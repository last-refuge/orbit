//! Release signatures: minisign over the tarball, verified with the public
//! key embedded in the binary.
//!
//! Policy: with no key embedded (an empty `minisign.pub`), signatures are
//! verified when present and ignored when absent. With a key embedded,
//! every release from [`SIGNED_SINCE`] on must carry a valid signature;
//! older releases are still accepted unsigned, since they were published
//! before signing existed.

use std::path::Path;

use minisign_verify::{PublicKey, Signature};

/// The repository's public key, `minisign.pub` at the root, as the
/// release workflow and `minisign -V` use it. Empty until the first key
/// is generated.
pub const PUBLIC_KEY_FILE: &str = include_str!("../../../minisign.pub");

/// The first version that must be signed once a key is embedded.
pub const SIGNED_SINCE: &str = "0.1.0-alpha.3";

/// The embedded key, if any.
pub fn public_key() -> Option<PublicKey> {
    public_key_from(PUBLIC_KEY_FILE)
}

pub fn public_key_from(file: &str) -> Option<PublicKey> {
    let base64 = file
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))?;
    PublicKey::from_base64(base64).ok()
}

/// Whether `version` must be signed under this policy.
pub fn required(key: Option<&PublicKey>, version: &str) -> bool {
    if key.is_none() {
        return false;
    }
    match (
        semver::Version::parse(version),
        semver::Version::parse(SIGNED_SINCE),
    ) {
        (Ok(v), Ok(since)) => v >= since,
        _ => true,
    }
}

/// Verifies `file` against `signature_file` (the `.minisig` text) with
/// `key`. Only the file's content and the key matter; the trusted comment
/// is checked for integrity too, as minisign does.
pub fn verify(key: &PublicKey, file: &Path, signature_file: &str) -> Result<(), String> {
    let signature =
        Signature::decode(signature_file).map_err(|e| format!("bad signature file: {e}"))?;
    let data = std::fs::read(file).map_err(|e| format!("reading {}: {e}", file.display()))?;
    key.verify(&data, &signature, false)
        .map_err(|e| format!("signature does not verify: {e}"))
}

/// The whole decision for one downloaded tarball: `signature_file` is the
/// downloaded `.minisig` when the release had one.
pub fn check(version: &str, file: &Path, signature_file: Option<&str>) -> Result<Verdict, String> {
    let key = public_key();
    match (key.as_ref(), signature_file) {
        (Some(key), Some(sig)) => verify(key, file, sig).map(|()| Verdict::Verified),
        (Some(_), None) if required(key.as_ref(), version) => Err(format!(
            "release {version} has no signature, and releases from {SIGNED_SINCE} on must be signed"
        )),
        (Some(_), None) => Ok(Verdict::UnsignedButOld),
        (None, Some(_)) => Ok(Verdict::NoKeyEmbedded),
        (None, None) => Ok(Verdict::NoKeyEmbedded),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Verified,
    /// Signed releases started later than this one.
    UnsignedButOld,
    /// This build carries no public key: nothing to verify against.
    NoKeyEmbedded,
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PUB: &str = include_str!("../tests/data/test.pub");
    const TEST_SIG: &str = include_str!("../tests/data/payload.txt.minisig");
    fn data(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name)
    }

    #[test]
    fn policy_without_a_key_accepts_unsigned_releases() {
        assert!(!required(None, "9.9.9"));
        assert!(public_key_from("").is_none());
        assert!(public_key_from("untrusted comment: only a comment\n").is_none());
    }

    #[test]
    fn policy_with_a_key_requires_signatures_from_signed_since_on() {
        let key = public_key_from(TEST_PUB).expect("test key parses");
        assert!(required(Some(&key), SIGNED_SINCE));
        assert!(required(Some(&key), "1.0.0"));
        assert!(!required(Some(&key), "0.1.0-alpha.1"));
    }

    #[test]
    fn verifies_a_minisign_signature_and_rejects_a_tampered_file() {
        let key = public_key_from(TEST_PUB).unwrap();
        verify(&key, &data("payload.txt"), TEST_SIG).expect("the signed payload verifies");
        let err = verify(&key, &data("tampered.txt"), TEST_SIG).unwrap_err();
        assert!(err.contains("does not verify"), "{err}");
        assert!(verify(&key, &data("payload.txt"), "not a signature").is_err());
        let other = public_key_from(PUBLIC_KEY_FILE);
        if let Some(other) = other {
            assert!(
                verify(&other, &data("payload.txt"), TEST_SIG).is_err(),
                "another key must not verify it"
            );
        }
    }

    #[test]
    fn the_embedded_release_key_parses_when_present() {
        if PUBLIC_KEY_FILE.trim().is_empty() {
            return;
        }
        assert!(
            public_key().is_some(),
            "minisign.pub at the repository root must be a minisign public key"
        );
    }
}
