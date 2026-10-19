//! `orbit events`: records Hyprland's event stream with titles redacted and
//! checks that every line parses (ORB-005). The recording becomes a replay
//! fixture for the parser and the state store.

use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use orbit_hyprland::{
    Instance,
    events::{parse, read_lines, redact_line},
    redact::Redactor,
};

pub struct Options {
    pub duration: Duration,
    pub record: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            duration: Duration::from_secs(10),
            record: None,
        }
    }
}

pub fn run(options: Options) -> Result<()> {
    let instance = Instance::from_env()?;
    let redactor = Redactor::default();
    let mut lines = Vec::new();
    let (mut used, mut ignored, mut malformed) = (0, 0, 0);
    read_lines(&instance, options.duration, |line| {
        let redacted = redact_line(line, &redactor);
        let status = match parse(line) {
            Ok(Some(_)) => {
                used += 1;
                "used"
            }
            Ok(None) => {
                ignored += 1;
                "ignored"
            }
            Err(error) => {
                malformed += 1;
                eprintln!("orbit events: {error}");
                "MALFORMED"
            }
        };
        println!("{status:<9} {redacted}");
        lines.push(redacted);
    })?;
    println!("{used} used, {ignored} ignored, {malformed} malformed");
    if let Some(path) = options.record {
        let mut text = lines.join("\n");
        text.push('\n');
        fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    }
    if malformed > 0 {
        anyhow::bail!("{malformed} malformed event line(s)");
    }
    Ok(())
}
