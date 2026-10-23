//! The request socket: one connection per query, `j/` prefix for JSON.
//! <https://wiki.hypr.land/IPC/>

use std::{
    env,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};

const TIMEOUT: Duration = Duration::from_secs(2);

/// Read-only snapshot queries. Control goes through [`Instance::dispatch`]
/// with a typed [`crate::control::Control`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    Version,
    Monitors,
    Workspaces,
    ActiveWorkspace,
    Clients,
    ActiveWindow,
    WorkspaceRules,
}

impl Query {
    pub const ALL: [Query; 7] = [
        Query::Version,
        Query::Monitors,
        Query::Workspaces,
        Query::ActiveWorkspace,
        Query::Clients,
        Query::ActiveWindow,
        Query::WorkspaceRules,
    ];

    pub fn command(self) -> &'static str {
        match self {
            Query::Version => "version",
            Query::Monitors => "monitors all",
            Query::Workspaces => "workspaces",
            Query::ActiveWorkspace => "activeworkspace",
            Query::Clients => "clients",
            Query::ActiveWindow => "activewindow",
            Query::WorkspaceRules => "workspacerules",
        }
    }

    /// A file-name-safe label for recorded fixtures.
    pub fn label(self) -> &'static str {
        match self {
            Query::Version => "version",
            Query::Monitors => "monitors",
            Query::Workspaces => "workspaces",
            Query::ActiveWorkspace => "activeworkspace",
            Query::Clients => "clients",
            Query::ActiveWindow => "activewindow",
            Query::WorkspaceRules => "workspacerules",
        }
    }
}

/// A running Hyprland instance, located by its signature.
#[derive(Clone, Debug)]
pub struct Instance {
    pub signature: String,
    pub dir: PathBuf,
}

impl Instance {
    /// Locates the instance this process's session belongs to.
    pub fn from_env() -> Result<Self> {
        let signature = env::var("HYPRLAND_INSTANCE_SIGNATURE")
            .context("HYPRLAND_INSTANCE_SIGNATURE is not set; is this a Hyprland session?")?;
        let runtime = env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        Self::locate(&signature, runtime.as_deref())
    }

    /// Current Hyprland keeps sockets under `$XDG_RUNTIME_DIR/hypr/<sig>`;
    /// releases before 0.40 used `/tmp/hypr/<sig>`.
    pub fn locate(signature: &str, runtime_dir: Option<&Path>) -> Result<Self> {
        let candidates = runtime_dir
            .map(|dir| dir.join("hypr").join(signature))
            .into_iter()
            .chain([Path::new("/tmp/hypr").join(signature)]);
        for dir in candidates {
            if dir.join(".socket.sock").exists() {
                return Ok(Self {
                    signature: signature.to_owned(),
                    dir,
                });
            }
        }
        bail!("no Hyprland request socket found for instance {signature}")
    }

    pub fn request_socket(&self) -> PathBuf {
        self.dir.join(".socket.sock")
    }

    pub fn event_socket(&self) -> PathBuf {
        self.dir.join(".socket2.sock")
    }

    /// Runs a query and returns the raw JSON text.
    pub fn query_raw(&self, query: Query) -> Result<String> {
        request(&self.request_socket(), &format!("j/{}", query.command()))
            .with_context(|| format!("hyprctl query `{}` failed", query.command()))
    }

    /// Sends one typed control request. The outer error is the socket
    /// failing; the inner one is Hyprland refusing the request.
    pub fn dispatch(
        &self,
        control: &crate::control::Control,
        syntax: crate::control::Syntax,
    ) -> Result<Result<(), String>> {
        let text = match control.encode(syntax) {
            Ok(text) => text,
            Err(refused) => return Ok(Err(refused)),
        };
        let command = if matches!(control, crate::control::Control::SwapWorkspaces { .. }) {
            "eval"
        } else {
            "dispatch"
        };
        let reply = request(&self.request_socket(), &format!("{command} {text}"))
            .context("hyprctl dispatch failed")?;
        Ok(crate::control::check_reply(&reply))
    }

    /// Reads one config option (`hyprctl getoption NAME`), as JSON.
    pub fn option(&self, name: &str) -> Result<serde_json::Value> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '_')
        {
            bail!("bad option name `{name}`");
        }
        let raw = request(&self.request_socket(), &format!("j/getoption {name}"))
            .with_context(|| format!("hyprctl getoption `{name}` failed"))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("`getoption {name}` did not return JSON"))
    }

    /// Runs a Lua chunk in the compositor (`hyprctl eval`), the way the 0.56
    /// Lua configuration is changed at runtime. The outer error is the
    /// socket failing; the inner one is Hyprland refusing the chunk.
    pub fn eval(&self, lua: &str) -> Result<Result<(), String>> {
        let reply = request(&self.request_socket(), &format!("eval {lua}"))
            .context("hyprctl eval failed")?;
        Ok(crate::control::check_reply(&reply))
    }

    /// The current key binds (`hyprctl binds -j`).
    pub fn binds(&self) -> Result<serde_json::Value> {
        let raw = request(&self.request_socket(), "j/binds").context("hyprctl binds failed")?;
        serde_json::from_str(&raw).context("`binds` did not return JSON")
    }

    pub fn query(&self, query: Query) -> Result<serde_json::Value> {
        let raw = self.query_raw(query)?;
        serde_json::from_str(&raw)
            .with_context(|| format!("`{}` did not return JSON", query.command()))
    }
}

fn request(socket: &Path, message: &str) -> Result<String> {
    let mut stream = UnixStream::connect(socket)
        .with_context(|| format!("connecting to {}", socket.display()))?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.write_all(message.as_bytes())?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use std::{os::unix::net::UnixListener, thread};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("orbit-hyprland-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn locate_prefers_runtime_dir() {
        let runtime = temp_dir("locate");
        let instance_dir = runtime.join("hypr").join("sig");
        std::fs::create_dir_all(&instance_dir).unwrap();
        std::fs::write(instance_dir.join(".socket.sock"), "").unwrap();

        let instance = Instance::locate("sig", Some(&runtime)).unwrap();
        assert_eq!(instance.dir, instance_dir);
        assert!(Instance::locate("other", Some(&runtime)).is_err());
    }

    #[test]
    fn queries_send_json_prefix_and_read_to_eof() {
        let runtime = temp_dir("query");
        let dir = runtime.join("hypr").join("sig");
        std::fs::create_dir_all(&dir).unwrap();
        let listener = UnixListener::bind(dir.join(".socket.sock")).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 64];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            stream.write_all(br#"{"version":"0.54.0"}"#).unwrap();
            request
        });

        let instance = Instance::locate("sig", Some(&runtime)).unwrap();
        let value = instance.query(Query::Version).unwrap();
        assert_eq!(value["version"], "0.54.0");
        assert_eq!(server.join().unwrap(), "j/version");
    }
}
