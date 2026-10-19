//! One check: fetch the releases list (conditionally), pick the newest on
//! the channel, remember it, back off after errors.

use crate::{
    client::Http,
    now,
    release::{Channel, parse_releases, select},
    state::{Latest, UpdateState},
};

/// Back-off after a failed or rate-limited check: 30 min, doubling, at
/// most a day.
const BACKOFF_BASE_SECS: u64 = 30 * 60;
const BACKOFF_MAX_SECS: u64 = 24 * 60 * 60;

#[derive(Clone, Debug)]
pub struct Checker {
    pub api_base: String,
    pub arch: String,
    pub channel: Channel,
    pub current: semver::Version,
    pub http: Http,
}

/// What a check concluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing newer than the running build (or the newer one is skipped).
    UpToDate,
    /// A newer release is on offer.
    Available(Latest),
    /// The check did not run: backing off until this time.
    Deferred { until: u64 },
}

impl Checker {
    /// Runs a check and updates `state`. `force` ignores the back-off and
    /// the ETag ("Check for updates" in the settings window).
    pub fn check(&self, state: &mut UpdateState, force: bool) -> Result<Outcome, String> {
        let now = now();
        if !force
            && let Some(until) = state.backoff_until
            && now < until
        {
            return Ok(Outcome::Deferred { until });
        }
        let url = format!(
            "{}/releases?per_page=10",
            self.api_base.trim_end_matches('/')
        );
        let etag = if force { None } else { state.etag.as_deref() };
        let fetched = match self.http.get(&url, etag) {
            Ok(fetched) => fetched,
            Err(error) => {
                self.back_off(state, now);
                return Err(error);
            }
        };
        match fetched.status {
            304 => {
                state.last_check = Some(now);
                state.failures = 0;
                state.backoff_until = None;
            }
            200 => {
                let releases = parse_releases(&fetched.body)?;
                state.latest = select(&releases, self.channel, &self.arch)
                    .as_ref()
                    .map(Latest::from);
                state.etag = fetched.etag;
                state.last_check = Some(now);
                state.failures = 0;
                state.backoff_until = None;
            }
            403 | 429 => {
                self.back_off(state, now);
                return Err("GitHub rate limit reached; will try again later".into());
            }
            other => {
                self.back_off(state, now);
                return Err(format!("GitHub answered {other}"));
            }
        }
        Ok(match state.available(&self.current) {
            Some(latest) => Outcome::Available(latest.clone()),
            None => Outcome::UpToDate,
        })
    }

    fn back_off(&self, state: &mut UpdateState, now: u64) {
        state.failures = state.failures.saturating_add(1);
        let secs = BACKOFF_BASE_SECS
            .saturating_mul(1u64 << state.failures.min(6))
            .min(BACKOFF_MAX_SECS);
        state.backoff_until = Some(now + secs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backs_off_exponentially_and_defers_until_then() {
        let checker = Checker {
            api_base: "http://127.0.0.1:1/x".into(),
            arch: "x86_64".into(),
            channel: Channel::Prerelease,
            current: semver::Version::parse("0.1.0").unwrap(),
            http: Http::new("test"),
        };
        let mut state = UpdateState::default();
        // Nothing listens on port 1: curl fails, so the check backs off.
        assert!(checker.check(&mut state, false).is_err());
        assert_eq!(state.failures, 1);
        let until = state.backoff_until.unwrap();
        assert!(until >= now() + BACKOFF_BASE_SECS * 2 - 5);
        assert_eq!(
            checker.check(&mut state, false).unwrap(),
            Outcome::Deferred { until }
        );
        // A forced check ignores the back-off (and fails the same way here).
        assert!(checker.check(&mut state, true).is_err());
        assert_eq!(state.failures, 2);
    }
}
