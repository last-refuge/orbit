//! Replacing the installed binary in place, and putting the previous one
//! back.
//!
//! Download into a temp directory beside the binary (same filesystem, so
//! the final step is one `rename`), verify the checksum, unpack, run the
//! new binary's `--version` as a sanity check, keep the current binary as
//! `orbit.prev`, rename the new one over it, refresh the desktop entry and
//! icons with the new binary. Any failure before the rename leaves the
//! install untouched.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    client::Http,
    install::{Install, InstallInfo},
    state::Latest,
};

/// What `apply` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub version: String,
    pub previous: PathBuf,
}

/// Verifies `latest` and installs it over `install`.
pub fn apply(install: &Install, latest: &Latest, http: &Http) -> Result<Applied, String> {
    let bin_dir = install
        .binary
        .parent()
        .ok_or("the binary has no parent directory")?;
    let work = bin_dir.join(format!(".orbit-update-{}", std::process::id()));
    let result = stage_and_swap(install, latest, http, &work);
    let _ = fs::remove_dir_all(&work);
    result
}

fn stage_and_swap(
    install: &Install,
    latest: &Latest,
    http: &Http,
    work: &Path,
) -> Result<Applied, String> {
    fs::create_dir_all(work).map_err(|e| format!("creating {}: {e}", work.display()))?;
    let name = latest
        .tarball_url
        .rsplit('/')
        .next()
        .filter(|n| n.ends_with(".tar.gz"))
        .ok_or_else(|| format!("odd tarball URL: {}", latest.tarball_url))?;
    let tarball = work.join(name);
    let sha = work.join(format!("{name}.sha256"));
    http.download(&latest.tarball_url, &tarball)?;
    http.download(&latest.sha256_url, &sha)?;
    verify_sha256(work, name)?;
    let out = Command::new("tar")
        .args(["-xzf"])
        .arg(&tarball)
        .args(["-C"])
        .arg(work)
        .output()
        .map_err(|e| format!("running tar: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "unpacking {name}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let new_binary = work.join(name.trim_end_matches(".tar.gz")).join("orbit");
    if !new_binary.is_file() {
        return Err("the tarball has no orbit binary where expected".into());
    }
    let reported = version_of(&new_binary)?;
    if reported != latest.version {
        return Err(format!(
            "the downloaded binary says it is {reported}, expected {}",
            latest.version
        ));
    }
    // The swap.
    let previous = install.previous();
    let _ = fs::remove_file(&previous);
    fs::rename(&install.binary, &previous)
        .map_err(|e| format!("keeping the current binary as {}: {e}", previous.display()))?;
    if let Err(e) = fs::rename(&new_binary, &install.binary) {
        let _ = fs::rename(&previous, &install.binary);
        return Err(format!("installing the new binary: {e}"));
    }
    // Desktop entry and icons from the new binary; not fatal.
    let assets = Command::new(&install.binary)
        .args(["assets", "install"])
        .arg(install.share())
        .output();
    if let Ok(out) = assets
        && !out.status.success()
    {
        log::warn!(
            "refreshing the desktop entry: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    if let Some(mut info) = InstallInfo::load() {
        info.installed_version = latest.version.clone();
        info.installed_at = crate::now();
        let _ = info.save();
    }
    Ok(Applied {
        version: latest.version.clone(),
        previous,
    })
}

/// Puts `orbit.prev` back as the binary. Returns the version it reports.
pub fn rollback(install: &Install) -> Result<String, String> {
    let previous = install.previous();
    if !previous.is_file() {
        return Err("no previous binary to go back to".into());
    }
    let version = version_of(&previous)?;
    let broken = install.binary.with_file_name("orbit.broken");
    let _ = fs::remove_file(&broken);
    fs::rename(&install.binary, &broken)
        .map_err(|e| format!("setting the current binary aside: {e}"))?;
    if let Err(e) = fs::rename(&previous, &install.binary) {
        let _ = fs::rename(&broken, &install.binary);
        return Err(format!("restoring the previous binary: {e}"));
    }
    let _ = fs::remove_file(&broken);
    if let Some(mut info) = InstallInfo::load() {
        info.installed_version = version.clone();
        let _ = info.save();
    }
    Ok(version)
}

/// `sha256sum -c` in `dir` against `NAME.sha256` (written by the release
/// workflow as `HASH  NAME`).
fn verify_sha256(dir: &Path, name: &str) -> Result<(), String> {
    let out = Command::new("sha256sum")
        .args(["--quiet", "-c"])
        .arg(format!("{name}.sha256"))
        .current_dir(dir)
        .output()
        .map_err(|e| format!("running sha256sum: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "checksum mismatch for {name}; the download was discarded"
        ))
    }
}

/// What `BINARY --version` prints after `orbit `.
pub fn version_of(binary: &Path) -> Result<String, String> {
    let out = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|e| format!("running {}: {e}", binary.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim()
        .strip_prefix("orbit ")
        .map(str::to_owned)
        .ok_or_else(|| format!("{} did not report a version", binary.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A stand-in binary: a shell script that answers `--version` and
    /// `assets install`.
    fn fake_binary(path: &Path, version: &str) {
        let mut f = fs::File::create(path).unwrap();
        writeln!(
            f,
            "#!/bin/sh\ncase \"$1\" in --version) echo \"orbit {version}\";; assets) exit 0;; esac"
        )
        .unwrap();
        let mut perm = fs::metadata(path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
        fs::set_permissions(path, perm).unwrap();
    }

    /// A release tarball in `dir` for `version`, with its checksum; returns
    /// the served file paths.
    fn fake_release(dir: &Path, version: &str, arch: &str) -> (PathBuf, PathBuf) {
        let name = format!("orbit-{version}-{arch}-unknown-linux-gnu");
        let root = dir.join(&name);
        fs::create_dir_all(&root).unwrap();
        fake_binary(&root.join("orbit"), version);
        let tarball = dir.join(format!("{name}.tar.gz"));
        assert!(
            Command::new("tar")
                .args(["-C"])
                .arg(dir)
                .args(["-czf"])
                .arg(&tarball)
                .arg(&name)
                .status()
                .unwrap()
                .success()
        );
        let out = Command::new("sha256sum")
            .arg(format!("{name}.tar.gz"))
            .current_dir(dir)
            .output()
            .unwrap();
        let sha = dir.join(format!("{name}.tar.gz.sha256"));
        fs::write(&sha, out.stdout).unwrap();
        fs::remove_dir_all(root).unwrap();
        (tarball, sha)
    }

    /// `Http` wants URLs; a `file://`-less test uses a loopback server.
    /// Here the download step is exercised through a tiny Python server.
    fn serve(dir: &Path) -> (std::process::Child, String) {
        let port = 18800 + (std::process::id() % 1000) as u16;
        let child = Command::new("python3")
            .args([
                "-I",
                "-m",
                "http.server",
                &port.to_string(),
                "--bind",
                "127.0.0.1",
            ])
            .current_dir(dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("python3 for the fixture server");
        std::thread::sleep(std::time::Duration::from_millis(700));
        (child, format!("http://127.0.0.1:{port}"))
    }

    #[test]
    fn applies_a_release_keeps_the_previous_binary_and_rolls_back() {
        let dir = std::env::temp_dir().join(format!("orbit-apply-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let prefix = dir.join("prefix");
        let bin = prefix.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let binary = bin.join("orbit");
        fake_binary(&binary, "0.1.0-alpha.1");
        let served = dir.join("served");
        fs::create_dir_all(&served).unwrap();
        let (tarball, sha) = fake_release(&served, "0.1.0-alpha.2", "x86_64");
        let (mut server, base) = serve(&served);
        let install = Install {
            prefix: prefix.clone(),
            binary: binary.clone(),
        };
        let latest = Latest {
            version: "0.1.0-alpha.2".into(),
            tag: "v0.1.0-alpha.2".into(),
            tarball_url: format!("{base}/{}", tarball.file_name().unwrap().to_string_lossy()),
            sha256_url: format!("{base}/{}", sha.file_name().unwrap().to_string_lossy()),
            ..Latest::default()
        };
        let http = Http::new("test");

        let applied = apply(&install, &latest, &http).unwrap();
        assert_eq!(applied.version, "0.1.0-alpha.2");
        assert_eq!(version_of(&binary).unwrap(), "0.1.0-alpha.2");
        assert_eq!(version_of(&install.previous()).unwrap(), "0.1.0-alpha.1");
        assert!(
            !bin.join(format!(".orbit-update-{}", std::process::id()))
                .exists(),
            "work dir cleaned"
        );

        // A tampered checksum is refused and nothing changes.
        fs::write(&sha, "deadbeef  nope.tar.gz\n").unwrap();
        let wrong = Latest {
            version: "0.1.0-alpha.3".into(),
            ..latest.clone()
        };
        let err = apply(&install, &wrong, &http).unwrap_err();
        assert!(err.contains("checksum"), "{err}");
        assert_eq!(version_of(&binary).unwrap(), "0.1.0-alpha.2");

        // A tarball whose binary reports the wrong version is refused too.
        let (tarball3, sha3) = fake_release(&served, "0.1.0-alpha.3", "x86_64");
        let lying = Latest {
            version: "0.1.0-alpha.4".into(),
            tarball_url: format!("{base}/{}", tarball3.file_name().unwrap().to_string_lossy()),
            sha256_url: format!("{base}/{}", sha3.file_name().unwrap().to_string_lossy()),
            ..Latest::default()
        };
        let err = apply(&install, &lying, &http).unwrap_err();
        assert!(err.contains("expected 0.1.0-alpha.4"), "{err}");
        assert_eq!(version_of(&binary).unwrap(), "0.1.0-alpha.2");

        // Rollback puts alpha.1 back.
        assert_eq!(rollback(&install).unwrap(), "0.1.0-alpha.1");
        assert_eq!(version_of(&binary).unwrap(), "0.1.0-alpha.1");
        assert!(!install.previous().exists());
        assert!(rollback(&install).is_err(), "nothing left to go back to");

        let _ = server.kill();
        let _ = fs::remove_dir_all(dir);
    }
}
