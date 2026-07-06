/// Universal identity representation returned by all authenticators.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// The nature of an authenticated principal — human vs non-human — used to
/// select the Cedar principal entity type ([`crate::authz::PrincipalKind`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IdentityKind {
    /// A human user (the default).
    #[default]
    User,
    /// A delegated agent acting on behalf of a user (RFC 8693 `act` token).
    Agent,
    /// A service / workload identity (client-credentials token).
    Service,
}

/// A successfully authenticated principal.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Identity {
    /// Unique subject identifier (user ID, service account ID, etc.)
    pub id: String,
    /// Whether this principal is a human user, a delegated agent, or a service.
    #[serde(default)]
    pub kind: IdentityKind,
    /// Provider-specific traits (e.g. Kratos identity traits).
    pub traits: Value,
    /// Public metadata attached to the identity.
    pub metadata_public: Value,
    /// Schema identifier (Kratos identity schema ID).
    pub schema_id: Option<String>,
    /// Session ID (for Kratos sessions).
    pub session_id: Option<String>,
    /// Authentication assurance level.
    pub aal: Option<String>,
    /// Arbitrary extra key/value data from the authenticator.
    pub extra: HashMap<String, String>,
}

impl Identity {
    /// Create a minimal anonymous identity with the given subject.
    pub fn anonymous(subject: impl Into<String>) -> Self {
        Self {
            id: subject.into(),
            ..Default::default()
        }
    }

    /// Convenience accessor for the `email` trait field.
    #[allow(dead_code)]
    pub fn email(&self) -> Option<&str> {
        self.traits.get("email").and_then(|v| v.as_str())
    }

    /// Serialize the identity into a JSON `Value` for use in template contexts.
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "traits": self.traits,
            "metadata_public": self.metadata_public,
            "schema_id": self.schema_id,
            "session_id": self.session_id,
            "aal": self.aal,
        })
    }
}

/// Namespaced claim the gateway stamps on tokens it mints for non-human
/// identities, so the principal kind cannot be spoofed by an upstream IdP's
/// ordinary claims (e.g. a user token that merely carries `client_id`/`azp`).
/// A normal OIDC provider will not emit this claim.
pub const FLINT_KIND_CLAIM: &str = "flint_kind";

impl Identity {
    /// Derive the [`IdentityKind`] from **trusted** token signals when the
    /// authenticator did not set it explicitly. Rules (fail-safe — default
    /// `User`, never escalates):
    /// - an explicit `self.kind` (set by the authenticator) is authoritative;
    /// - the gateway-stamped `flint_kind` claim (`agent`|`service`) wins — this
    ///   only appears on tokens flint-gate itself minted;
    /// - an `act` (RFC 8693 actor) claim ⇒ **Agent** (delegation-specific; a
    ///   normal user token does not carry it);
    /// - otherwise **User**.
    ///
    /// A bare `client_id` claim is deliberately NOT treated as a Service signal:
    /// many OIDC access tokens carry `client_id`/`azp` for ordinary users, so
    /// inferring Service from it would let a user reach a Service-scoped policy
    /// (privilege escalation). Service classification requires the gateway's own
    /// `flint_kind` marker.
    pub fn derived_kind(&self) -> IdentityKind {
        if self.kind != IdentityKind::User {
            return self.kind;
        }
        match self.metadata_public.get(FLINT_KIND_CLAIM).and_then(Value::as_str) {
            Some("agent") => return IdentityKind::Agent,
            Some("service") => return IdentityKind::Service,
            _ => {}
        }
        // The `act` claim promotes to Agent ONLY for token-derived identities,
        // never for a Kratos session: Kratos `metadata_public` can be admin- or
        // (in some deployments) self-service-writable, so an `act` field there is
        // not a trustworthy delegation signal. A `session_id` marks a Kratos
        // identity — skip the `act` fallback for it. The gateway-signed
        // `flint_kind` marker above is unaffected (Kratos never sets it).
        if self.session_id.is_none() && self.metadata_public.get("act").is_some() {
            IdentityKind::Agent
        } else {
            IdentityKind::User
        }
    }
}

/// Map an [`Identity`] to the Cedar [`crate::authz::PrincipalKind`] it authorizes
/// as, using the identity's explicit or claim-derived [`IdentityKind`].
pub fn principal_kind_for(identity: &Identity) -> crate::authz::PrincipalKind {
    match identity.derived_kind() {
        IdentityKind::User => crate::authz::PrincipalKind::User,
        IdentityKind::Agent => crate::authz::PrincipalKind::Agent,
        IdentityKind::Service => crate::authz::PrincipalKind::Service,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn derived_kind_defaults_to_user() {
        assert_eq!(Identity::default().derived_kind(), IdentityKind::User);
    }

    #[test]
    fn derived_kind_agent_from_act_claim() {
        let id = Identity {
            metadata_public: json!({ "act": { "sub": "user-1" } }),
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), IdentityKind::Agent);
        assert_eq!(
            principal_kind_for(&id),
            crate::authz::PrincipalKind::Agent
        );
    }

    #[test]
    fn derived_kind_service_from_flint_kind_marker() {
        // Service classification requires the gateway's own `flint_kind` marker,
        // NOT a bare client_id (which normal OIDC user tokens carry).
        let id = Identity {
            metadata_public: json!({ "flint_kind": "service", "client_id": "svc-7" }),
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), IdentityKind::Service);
        assert_eq!(
            principal_kind_for(&id),
            crate::authz::PrincipalKind::Service
        );
    }

    #[test]
    fn bare_client_id_does_not_escalate_to_service() {
        // A user token that merely carries client_id/azp stays a User — no
        // privilege escalation into a Service-scoped policy.
        let id = Identity {
            metadata_public: json!({ "client_id": "some-oidc-client" }),
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), IdentityKind::User);
    }

    #[test]
    fn kratos_session_identity_with_act_trait_stays_user() {
        // A Kratos identity (session_id set) must NOT self-promote to Agent via
        // an `act` field in its metadata_public — that field is not a trusted
        // delegation signal for a session-authenticated user.
        let id = Identity {
            session_id: Some("kratos-sess-1".into()),
            metadata_public: json!({ "act": { "sub": "someone" } }),
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), IdentityKind::User);
        assert_eq!(principal_kind_for(&id), crate::authz::PrincipalKind::User);
    }

    #[test]
    fn token_identity_with_act_but_no_session_is_agent() {
        // The gateway's own delegated tokens have no session_id → `act` still
        // promotes to Agent (unchanged behavior).
        let id = Identity {
            session_id: None,
            metadata_public: json!({ "act": { "sub": "u" } }),
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), IdentityKind::Agent);
    }

    #[test]
    fn kratos_session_flint_kind_still_trusted() {
        // The gateway-signed flint_kind marker is trusted even on a session
        // identity (Kratos never sets it, so its presence means gateway-minted).
        let id = Identity {
            session_id: Some("s".into()),
            metadata_public: json!({ "flint_kind": "service" }),
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), IdentityKind::Service);
    }

    #[test]
    fn explicit_kind_overrides_claim_derivation() {
        // An authenticator that set kind explicitly is authoritative.
        let id = Identity {
            kind: IdentityKind::Service,
            metadata_public: json!({ "act": { "sub": "u" } }), // would say Agent
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), IdentityKind::Service);
    }

    #[test]
    fn act_claim_takes_precedence_over_client_id() {
        // A delegated agent token that also carries client_id is an Agent.
        let id = Identity {
            metadata_public: json!({ "act": { "sub": "u" }, "client_id": "c" }),
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), IdentityKind::Agent);
    }

    #[test]
    fn anonymous_identity() {
        let id = Identity::anonymous("anon");
        assert_eq!(id.id, "anon");
        assert!(id.email().is_none());
    }

    #[test]
    fn identity_to_value() {
        let id = Identity {
            id: "user-1".to_string(),
            traits: json!({"email": "a@b.com"}),
            ..Default::default()
        };
        let v = id.to_value();
        assert_eq!(v["id"], "user-1");
        assert_eq!(v["traits"]["email"], "a@b.com");
    }

    #[test]
    fn email_shortcut() {
        let id = Identity {
            traits: json!({"email": "hi@example.com"}),
            ..Default::default()
        };
        assert_eq!(id.email(), Some("hi@example.com"));
    }
}
