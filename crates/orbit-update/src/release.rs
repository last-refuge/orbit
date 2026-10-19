//! GitHub releases as the API lists them, and which one to offer.

use semver::Version;
use serde::Deserialize;

/// Which releases count: every one, or only those not marked pre-release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Prerelease,
    Stable,
}

impl Channel {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "prerelease" => Some(Channel::Prerelease),
            "stable" => Some(Channel::Stable),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Channel::Prerelease => "prerelease",
            Channel::Stable => "stable",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub url: String,
    #[serde(default)]
    pub size: u64,
}

/// One release as GitHub's `/releases` lists it (the fields Orbit reads).
#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    /// The version the tag names (`v0.1.0-alpha.2` → `0.1.0-alpha.2`).
    pub fn version(&self) -> Option<Version> {
        Version::parse(self.tag_name.trim_start_matches('v')).ok()
    }
}

/// A release Orbit could install: newest on the channel, with a tarball and
/// a checksum for this architecture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub version: Version,
    pub tag: String,
    pub html_url: String,
    pub notes: String,
    pub published_at: String,
    pub tarball: Asset,
    pub sha256: Asset,
    pub signature: Option<Asset>,
}

pub fn parse_releases(json: &str) -> Result<Vec<Release>, String> {
    serde_json::from_str(json).map_err(|e| format!("releases JSON: {e}"))
}

/// The tarball's name for `version` on `arch`, as the release workflow
/// packages it.
pub fn asset_name(version: &Version, arch: &str) -> String {
    format!("orbit-{version}-{arch}-unknown-linux-gnu.tar.gz")
}

/// The newest release on `channel` that has both assets for `arch`.
/// Drafts never count; on `Stable`, releases marked pre-release do not
/// either (a tag with a pre-release suffix counts as what GitHub says).
pub fn select(releases: &[Release], channel: Channel, arch: &str) -> Option<Candidate> {
    let mut best: Option<Candidate> = None;
    for release in releases {
        if release.draft || (channel == Channel::Stable && release.prerelease) {
            continue;
        }
        let Some(version) = release.version() else {
            continue;
        };
        if best.as_ref().is_some_and(|b| b.version >= version) {
            continue;
        }
        let name = asset_name(&version, arch);
        let tarball = release.assets.iter().find(|a| a.name == name);
        let sha256 = release
            .assets
            .iter()
            .find(|a| a.name == format!("{name}.sha256"));
        let (Some(tarball), Some(sha256)) = (tarball, sha256) else {
            continue;
        };
        let signature = release
            .assets
            .iter()
            .find(|a| a.name == format!("{name}.minisig"))
            .cloned();
        best = Some(Candidate {
            version,
            tag: release.tag_name.clone(),
            html_url: release.html_url.clone(),
            notes: release.body.clone().unwrap_or_default(),
            published_at: release.published_at.clone().unwrap_or_default(),
            tarball: tarball.clone(),
            sha256: sha256.clone(),
            signature,
        });
    }
    best
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const FIXTURE: &str = r#"[
      {"tag_name": "v0.2.0", "draft": true, "prerelease": false, "html_url": "https://example/0.2.0",
       "assets": [{"name": "orbit-0.2.0-x86_64-unknown-linux-gnu.tar.gz", "browser_download_url": "https://dl/0.2.0.tgz"},
                  {"name": "orbit-0.2.0-x86_64-unknown-linux-gnu.tar.gz.sha256", "browser_download_url": "https://dl/0.2.0.sha"}]},
      {"tag_name": "v0.1.0-alpha.2", "prerelease": true, "html_url": "https://example/a2", "body": "Second alpha",
       "published_at": "2026-10-20T00:00:00Z",
       "assets": [{"name": "orbit-0.1.0-alpha.2-x86_64-unknown-linux-gnu.tar.gz", "browser_download_url": "https://dl/a2.tgz", "size": 10},
                  {"name": "orbit-0.1.0-alpha.2-x86_64-unknown-linux-gnu.tar.gz.sha256", "browser_download_url": "https://dl/a2.sha"},
                  {"name": "orbit-0.1.0-alpha.2-x86_64-unknown-linux-gnu.tar.gz.minisig", "browser_download_url": "https://dl/a2.sig"}]},
      {"tag_name": "v0.1.0-alpha.1", "prerelease": true, "html_url": "https://example/a1",
       "assets": [{"name": "orbit-0.1.0-alpha.1-x86_64-unknown-linux-gnu.tar.gz", "browser_download_url": "https://dl/a1.tgz"},
                  {"name": "orbit-0.1.0-alpha.1-x86_64-unknown-linux-gnu.tar.gz.sha256", "browser_download_url": "https://dl/a1.sha"},
                  {"name": "orbit-0.1.0-alpha.1-aarch64-unknown-linux-gnu.tar.gz", "browser_download_url": "https://dl/a1-arm.tgz"},
                  {"name": "orbit-0.1.0-alpha.1-aarch64-unknown-linux-gnu.tar.gz.sha256", "browser_download_url": "https://dl/a1-arm.sha"}]},
      {"tag_name": "v0.0.9", "prerelease": false, "html_url": "https://example/0.0.9",
       "assets": [{"name": "orbit-0.0.9-x86_64-unknown-linux-gnu.tar.gz", "browser_download_url": "https://dl/0.0.9.tgz"},
                  {"name": "orbit-0.0.9-x86_64-unknown-linux-gnu.tar.gz.sha256", "browser_download_url": "https://dl/0.0.9.sha"}]},
      {"tag_name": "not-a-version", "assets": []}
    ]"#;

    #[test]
    fn selects_the_newest_with_assets_per_channel_and_arch() {
        let releases = parse_releases(FIXTURE).unwrap();
        let pre = select(&releases, Channel::Prerelease, "x86_64").unwrap();
        assert_eq!(pre.tag, "v0.1.0-alpha.2", "drafts never count");
        assert_eq!(pre.notes, "Second alpha");
        assert_eq!(pre.tarball.url, "https://dl/a2.tgz");
        assert_eq!(pre.sha256.url, "https://dl/a2.sha");
        assert_eq!(
            pre.signature.as_ref().map(|a| a.url.as_str()),
            Some("https://dl/a2.sig")
        );
        let stable = select(&releases, Channel::Stable, "x86_64").unwrap();
        assert_eq!(stable.tag, "v0.0.9", "stable skips pre-releases");
        let arm = select(&releases, Channel::Prerelease, "aarch64").unwrap();
        assert_eq!(
            arm.tag, "v0.1.0-alpha.1",
            "only releases with assets for the arch"
        );
        assert!(select(&releases, Channel::Stable, "aarch64").is_none());
    }

    #[test]
    fn prerelease_ordering_is_semver() {
        let a1 = Version::parse("0.1.0-alpha.1").unwrap();
        let a2 = Version::parse("0.1.0-alpha.2").unwrap();
        let a10 = Version::parse("0.1.0-alpha.10").unwrap();
        let final_ = Version::parse("0.1.0").unwrap();
        assert!(a1 < a2 && a2 < a10 && a10 < final_);
        assert_eq!(
            asset_name(&a2, "x86_64"),
            "orbit-0.1.0-alpha.2-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(Channel::parse("stable"), Some(Channel::Stable));
        assert_eq!(Channel::parse("nightly"), None);
    }
}
