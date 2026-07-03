//! PKCE (RFC 7636) `S256` code-challenge verification.
//!
//! Used by the MCP auth flow when this server terminates an authorization-code
//! exchange. flint-gate is a **Resource Server**, not an Authorization Server,
//! so there is no code-exchange endpoint on the RS surface today — this helper
//! is exposed `pub(crate)` and unit-tested against the RFC 7636 vectors so that
//! the AS-facing exchange seam (added by a later change) can call it directly
//! without re-deriving the transform.
//!
//! The `S256` method is defined as:
//! `code_challenge = BASE64URL-ENCODE(SHA256(ASCII(code_verifier)))`
//! where BASE64URL is url-safe, no padding (RFC 7636 §4.2).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

/// Verify a PKCE `S256` challenge.
///
/// Returns `true` iff `base64url_nopad(SHA256(verifier)) == challenge`.
///
/// Comparison is a plain byte-equality of the two base64url strings; both sides
/// are fixed-length (43 chars) derived values, and the challenge is public
/// (transmitted in the clear during the authorization request), so this is not
/// a secret-comparison timing-attack surface. Fails CLOSED — any mismatch,
/// including an empty verifier or malformed challenge, returns `false`.
#[allow(dead_code)] // Consumed by the AS-facing code-exchange seam (later change).
pub(crate) fn verify_pkce_s256(verifier: &str, challenge: &str) -> bool {
    if verifier.is_empty() || challenge.is_empty() {
        return false;
    }
    let digest = Sha256::digest(verifier.as_bytes());
    let computed = URL_SAFE_NO_PAD.encode(digest);
    computed == challenge
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7636 Appendix B test vector.
    /// verifier  = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
    /// challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    #[test]
    fn rfc7636_appendix_b_vector_matches() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(verify_pkce_s256(verifier, challenge));
    }

    #[test]
    fn wrong_verifier_fails() {
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(!verify_pkce_s256("not-the-verifier", challenge));
    }

    #[test]
    fn tampered_challenge_fails() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        // Flip the last character of the valid challenge.
        let tampered = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cX";
        assert!(!verify_pkce_s256(verifier, tampered));
    }

    #[test]
    fn empty_inputs_fail_closed() {
        assert!(!verify_pkce_s256("", "anything"));
        assert!(!verify_pkce_s256("verifier", ""));
        assert!(!verify_pkce_s256("", ""));
    }

    #[test]
    fn padding_is_stripped() {
        // A short verifier whose SHA-256 base64 would normally carry '=' padding;
        // URL_SAFE_NO_PAD must not emit any '='.
        let digest = Sha256::digest(b"abc");
        let computed = URL_SAFE_NO_PAD.encode(digest);
        assert!(!computed.contains('='), "no padding in S256 challenge");
        assert!(verify_pkce_s256("abc", &computed));
    }
}
