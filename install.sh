#!/usr/bin/env bash
# Orbit installer: the workspace overview for Hyprland.
#
#   curl -fsSL https://raw.githubusercontent.com/last-refuge/orbit/main/install.sh | bash
#
# Options (after `bash -s --` when piped):
#   --version vX.Y.Z   install that release instead of the latest
#   --prefix DIR       install under DIR (default ~/.local: bin/orbit, share/…)
#   --from-source      build with cargo instead of downloading a release
#   --source DIR       build from an existing checkout (implies --from-source)
#   --uninstall        remove the binary, desktop entry, icons and login unit
#   --purge            with --uninstall: also remove ~/.config/orbit and the
#                      generated ~/.config/hypr/orbit.lua
#   --help
#
# Never uses sudo. Never edits your Hyprland configuration: the one line
# Orbit needs in hyprland.lua is printed at the end for you to add.

set -euo pipefail

REPO=${ORBIT_REPO:-last-refuge/orbit}
API_BASE=${ORBIT_API_BASE:-https://api.github.com/repos/$REPO}
DOWNLOAD_BASE=${ORBIT_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download}
PREFIX=${ORBIT_PREFIX:-$HOME/.local}
CONFIG_HOME=${XDG_CONFIG_HOME:-$HOME/.config}
CACHE_HOME=${XDG_CACHE_HOME:-$HOME/.cache}

version=""
from_source=false
source_dir=""
uninstall=false
purge=false

say() { printf '\033[1;35morbit\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31merror\033[0m %s\n' "$*" >&2; exit 1; }

usage() { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
    case $1 in
        --version) version=${2:?--version needs a tag like v0.1.0}; shift ;;
        --version=*) version=${1#*=} ;;
        --prefix) PREFIX=${2:?--prefix needs a directory}; shift ;;
        --prefix=*) PREFIX=${1#*=} ;;
        --from-source) from_source=true ;;
        --source) source_dir=${2:?--source needs a directory}; from_source=true; shift ;;
        --source=*) source_dir=${1#*=}; from_source=true ;;
        --uninstall) uninstall=true ;;
        --purge) purge=true ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option: $1 (try --help)" ;;
    esac
    shift
done

bin_dir=$PREFIX/bin
share_dir=$PREFIX/share
binary=$bin_dir/orbit
unit=$CONFIG_HOME/systemd/user/orbit.service

# --- uninstall ---------------------------------------------------------------

do_uninstall() {
    say "removing Orbit from $PREFIX"
    if [ -x "$binary" ]; then
        "$binary" quit >/dev/null 2>&1 || true
    fi
    if [ -f "$unit" ]; then
        systemctl --user disable --now orbit.service >/dev/null 2>&1 || true
        rm -f "$unit"
        systemctl --user daemon-reload >/dev/null 2>&1 || true
        say "removed the login unit"
    fi
    if [ -x "$binary" ]; then
        "$binary" assets remove "$share_dir" >/dev/null 2>&1 || true
    fi
    rm -f "$share_dir/applications/orbit.desktop"
    for size in 64 128 256 512; do
        rm -f "$share_dir/icons/hicolor/${size}x${size}/apps/orbit.png"
    done
    rm -f "$binary"
    say "removed the binary, desktop entry and icons"
    if $purge; then
        rm -rf "$CONFIG_HOME/orbit"
        rm -f "$CONFIG_HOME/hypr/orbit.lua"
        say "removed $CONFIG_HOME/orbit and $CONFIG_HOME/hypr/orbit.lua"
        if [ -f "$CONFIG_HOME/hypr/hyprland.lua" ] && grep -q 'require("hypr.orbit")' "$CONFIG_HOME/hypr/hyprland.lua" 2>/dev/null; then
            warn "hyprland.lua still has require(\"hypr.orbit\"); remove that line yourself (this script never edits it)"
        fi
    else
        say "kept your settings ($CONFIG_HOME/orbit) and $CONFIG_HOME/hypr/orbit.lua; add --purge to remove them"
    fi
}

if $uninstall; then
    do_uninstall
    exit 0
fi

# --- preflight ---------------------------------------------------------------

[ "$(uname -s)" = Linux ] || die "Orbit runs on Linux (Hyprland); this is $(uname -s)"
case $(uname -m) in
    x86_64|aarch64) arch=$(uname -m) ;;
    *) die "no build for $(uname -m); use --from-source" ;;
esac
for tool in curl tar sha256sum; do
    command -v "$tool" >/dev/null || die "$tool is needed"
done
if [ -z "${WAYLAND_DISPLAY:-}" ]; then
    warn "no Wayland session in this shell; installing anyway (Orbit runs under Hyprland 0.56+)"
elif [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
    warn "this does not look like a Hyprland session; installing anyway (Orbit needs Hyprland 0.56+)"
fi
mkdir -p "$bin_dir" "$share_dir"

# --- a release tarball -------------------------------------------------------

latest_tag() {
    curl -fsSL -H 'Accept: application/vnd.github+json' "$API_BASE/releases/latest" 2>/dev/null \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1
}

install_release() {
    local tag=$1 name tmp
    name=orbit-${tag#v}-$arch-unknown-linux-gnu
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' RETURN
    say "downloading $name.tar.gz"
    curl -fsSL -o "$tmp/$name.tar.gz" "$DOWNLOAD_BASE/$tag/$name.tar.gz" || return 1
    curl -fsSL -o "$tmp/$name.tar.gz.sha256" "$DOWNLOAD_BASE/$tag/$name.tar.gz.sha256" || return 1
    (cd "$tmp" && sha256sum --quiet -c "$name.tar.gz.sha256") || die "checksum mismatch for $name.tar.gz"
    tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
    [ -x "$tmp/$name/orbit" ] || die "the tarball has no orbit binary"
    install -m 755 "$tmp/$name/orbit" "$binary"
    say "installed $binary ($tag)"
}

# --- from source -------------------------------------------------------------

install_from_source() {
    local src=$source_dir
    command -v cargo >/dev/null || die "cargo is needed to build from source: install rustup (https://rustup.rs) and run again
  Arch/Omarchy: pacman -S rustup base-devel libxkbcommon wayland vulkan-icd-loader fontconfig freetype2 zstd; rustup default stable"
    if [ -z "$src" ]; then
        command -v git >/dev/null || die "git is needed to fetch the source"
        src=$CACHE_HOME/orbit/src
        if [ -d "$src/.git" ]; then
            say "updating $src"
            git -C "$src" fetch -q --tags origin
        else
            say "cloning https://github.com/$REPO.git into $src"
            mkdir -p "$(dirname "$src")"
            git clone -q "https://github.com/$REPO.git" "$src"
        fi
        if [ -n "$version" ]; then
            git -C "$src" checkout -q "$version"
        else
            git -C "$src" checkout -q main && git -C "$src" pull -q --ff-only origin main
        fi
    fi
    [ -f "$src/Cargo.toml" ] || die "$src is not an Orbit checkout"
    say "building in $src (release profile; the first build takes a few minutes)"
    if ! (cd "$src" && cargo build --release --locked 2>&1 | tail -n 20); then
        die "the build failed. Development packages Orbit links against (Arch/Omarchy names):
  libxkbcommon wayland vulkan-icd-loader fontconfig freetype2 zstd"
    fi
    install -m 755 "$src/target/release/orbit" "$binary"
    say "installed $binary ($("$binary" --version))"
}

if $from_source; then
    install_from_source
else
    tag=$version
    if [ -z "$tag" ]; then
        tag=$(latest_tag || true)
    fi
    if [ -n "$tag" ] && install_release "$tag"; then
        :
    else
        if [ -n "$tag" ]; then
            warn "no release asset for $tag on $arch; building from source instead"
        else
            warn "no published release yet; building from source instead"
        fi
        install_from_source
    fi
fi

# --- desktop entry, icons, path ---------------------------------------------

"$binary" assets install "$share_dir" >/dev/null
command -v update-desktop-database >/dev/null && update-desktop-database -q "$share_dir/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$share_dir/icons/hicolor" 2>/dev/null || true
say "desktop entry and icons under $share_dir"

case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) warn "$bin_dir is not on your PATH; add it (e.g. in ~/.bashrc): export PATH=\"$bin_dir:\$PATH\"" ;;
esac

# --- next steps --------------------------------------------------------------

shortcut='Super+`'
if [ -f "$CONFIG_HOME/orbit/config.toml" ]; then
    shortcut=$("$binary" config get shortcut.key 2>/dev/null | sed 's/^grave$/`/') && shortcut="Super+$shortcut"
fi

cat <<NEXT

  Orbit is installed. Next:

  1. Start it:                 orbit daemon &
     (or at every login:       orbit config set startup.launch_at_login true)
  2. Press $shortcut to open the overview, or rest the pointer in a hot
     corner once you turn one on in the settings (Ctrl+, inside the overview).
  3. To keep the shortcut across Hyprland restarts, add this one line to
     ~/.config/hypr/hyprland.lua (the settings window can do it for you):

         require("hypr.orbit")

  Check your session:          orbit doctor
  Remove it later:             curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | bash -s -- --uninstall

NEXT
