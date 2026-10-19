//! HTTP through `curl` as a subprocess: HTTPS only, three GitHub hosts, or
//! a loopback `http://` server for the checks. Conditional requests carry
//! the stored ETag so an unchanged answer costs no rate limit.

use std::{
    path::Path,
    process::{Command, Stdio},
};

/// Hosts a release check or download may contact over HTTPS.
pub const ALLOWED_HOSTS: [&str; 3] = [
    "api.github.com",
    "github.com",
    "objects.githubusercontent.com",
];

#[derive(Clone, Debug, Default)]
pub struct Http {
    /// The `User-Agent` curl sends (GitHub requires one).
    pub user_agent: String,
}

/// What a request came back with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub status: u16,
    pub etag: Option<String>,
    pub body: String,
}

impl Http {
    pub fn new(version: &str) -> Self {
        Self {
            user_agent: format!("orbit/{version} (+https://github.com/last-refuge/orbit)"),
        }
    }

    /// Refuses anything but `https://` to an allowed host, or `http://` to
    /// the loopback address (the fixture server in the checks).
    pub fn allowed(url: &str) -> Result<(), String> {
        let (scheme, rest) = url
            .split_once("://")
            .ok_or_else(|| format!("not a URL: {url}"))?;
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = host.rsplit('@').next().unwrap_or(host);
        let host_only = host.split(':').next().unwrap_or(host);
        match scheme {
            "https" if ALLOWED_HOSTS.contains(&host_only) => Ok(()),
            "http" if host_only == "127.0.0.1" || host_only == "localhost" => Ok(()),
            _ => Err(format!(
                "refusing to contact {scheme}://{host}: not a GitHub host"
            )),
        }
    }

    fn curl(&self) -> Command {
        let mut cmd = Command::new("curl");
        cmd.args([
            "-sS",
            "-L",
            "--proto",
            "=https,http",
            "--proto-redir",
            "=https,http",
            "--max-redirs",
            "5",
            "--connect-timeout",
            "10",
            "--max-time",
            "120",
            "-A",
            &self.user_agent,
        ])
        .stdin(Stdio::null());
        cmd
    }

    /// Fetches a small text document (the releases list). `etag` makes it a
    /// conditional request; a 304 comes back with an empty body.
    pub fn get(&self, url: &str, etag: Option<&str>) -> Result<Fetched, String> {
        Self::allowed(url)?;
        let dir = std::env::temp_dir().join(format!("orbit-http-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
        let headers = dir.join("headers");
        let body = dir.join("body");
        let mut cmd = self.curl();
        cmd.args(["-H", "Accept: application/vnd.github+json"]);
        if let Some(etag) = etag {
            cmd.args(["-H", &format!("If-None-Match: {etag}")]);
        }
        cmd.args(["-D"])
            .arg(&headers)
            .args(["-o"])
            .arg(&body)
            .args(["-w", "%{http_code}"])
            .arg(url);
        let out = cmd.output().map_err(|e| format!("running curl: {e}"))?;
        let status: u16 = String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        if !out.status.success() && status == 0 {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!(
                "curl failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let header_text = std::fs::read_to_string(&headers).unwrap_or_default();
        let body_text = std::fs::read_to_string(&body).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        // With redirects curl writes every response's headers; the last block wins.
        let etag = header_text
            .lines()
            .filter_map(|l| l.split_once(':'))
            .filter(|(k, _)| k.eq_ignore_ascii_case("etag"))
            .map(|(_, v)| v.trim().to_owned())
            .next_back();
        Ok(Fetched {
            status,
            etag,
            body: body_text,
        })
    }

    /// Downloads `url` to `dest`, resuming a partial file.
    pub fn download(&self, url: &str, dest: &Path) -> Result<(), String> {
        Self::allowed(url)?;
        let mut cmd = self.curl();
        cmd.args(["-f", "-C", "-", "-o"]).arg(dest).arg(url);
        let out = cmd.output().map_err(|e| format!("running curl: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "downloading {url}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_github_over_https_or_loopback_over_http() {
        assert!(Http::allowed("https://api.github.com/repos/x/releases").is_ok());
        assert!(
            Http::allowed("https://github.com/last-refuge/orbit/releases/download/v1/x.tar.gz")
                .is_ok()
        );
        assert!(Http::allowed("https://objects.githubusercontent.com/abc?x=1").is_ok());
        assert!(Http::allowed("http://127.0.0.1:18731/releases").is_ok());
        assert!(Http::allowed("http://localhost:1/x").is_ok());
        assert!(
            Http::allowed("http://api.github.com/x").is_err(),
            "no plain http to GitHub"
        );
        assert!(Http::allowed("https://evil.example/x").is_err());
        assert!(Http::allowed("https://api.github.com.evil.example/x").is_err());
        assert!(Http::allowed("https://user@evil.example/x").is_err());
        assert!(Http::allowed("ftp://github.com/x").is_err());
    }
}
