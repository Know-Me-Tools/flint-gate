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
    ///   is trustworthy because the JWKS verifiers (`jwt_verify`, `mcp`) STRIP any
    ///   inbound `flint_kind`, so it can only be present on a token flint-gate
    ///   itself minted (a federated IdP cannot forge it to escalate);
    /// - an `act` (RFC 8693 actor) claim ⇒ **Agent** (delegation-specific; a
    ///   normal user token does not carry it). This is **gateway-side** and
    ///   **IdM-agnostic**: a delegated token minted by ANY JWKS provider — the
    ///   gateway-local exchange OR an Ory Hydra `delegate_to_hydra` exchange —
    ///   carries `act`, so it classifies as Agent without the gateway rewriting
    ///   the token or requiring a Hydra-side claim mapper (federate, never an IdP);
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
        //
        // The `act` value must be a **well-formed actor object** (RFC 8693 §4.1:
        // a JSON object identifying the current actor). A non-object `act`
        // (`null`, `false`, `""`, `[]`, …) is NOT a valid delegation signal and
        // must not promote to Agent — this tightens the spoof surface.
        if self.session_id.is_none()
            && self
                .metadata_public
                .get("act")
                .is_some_and(is_well_formed_act)
        {
            IdentityKind::Agent
        } else {
            IdentityKind::User
        }
    }
}

/// Whether a claim value is a well-formed RFC 8693 §4.1 `act` (actor) claim: a
/// JSON object carrying a non-empty string `sub` (the actor's identifier). A
/// non-object, an object without a usable `sub`, or an empty `act` is not a
/// trustworthy delegation signal and must not classify the token as an Agent.
fn is_well_formed_act(act: &Value) -> bool {
    act.as_object().is_some_and(|o| {
        o.get("sub")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty())
    })
}

/// Remove any `flint_kind` key from a metadata object sourced from an **external
/// / federated** authenticator (JWKS IdM, Kratos session). `flint_kind` is the
/// gateway's own spoof-resistant principal-kind marker and is trustworthy ONLY on
/// tokens the gateway itself minted; an external IdP or a self-service identity
/// could otherwise carry `flint_kind: agent`/`service` and escalate to a
/// non-human principal via [`derived_kind`](Identity::derived_kind). Every
/// authenticator that builds an `Identity` from untrusted upstream metadata MUST
/// call this. Delegated agents re-enter via their RFC 8693 `act` claim, not a
/// surviving `flint_kind`.
///
/// NOTE (re-entry asymmetry): only the `act`-based **Agent** signal survives a
/// JWKS round-trip. A gateway-minted **Service** (client-credentials) token
/// carries `flint_kind: service` but no `act`, so if re-verified through a JWKS
/// authenticator its Service kind is stripped and it classifies as `User` — a
/// fail-safe downgrade (never an escalation). Service kind is authoritative at
/// its mint boundary (explicit `kind`), not on JWKS re-entry.
pub fn strip_untrusted_kind(metadata: &mut Value) {
    if let Some(obj) = metadata.as_object_mut() {
        obj.remove(FLINT_KIND_CLAIM);
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
    fn strip_untrusted_kind_removes_flint_kind_from_federated_metadata() {
        // `flint_kind` on federated metadata (a Kratos session, an IdP token) is
        // UNTRUSTED — the authenticator strips it, so it can never reach
        // derived_kind to escalate a human into a non-human principal.
        let mut meta = json!({ "flint_kind": "service", "org": "acme" });
        strip_untrusted_kind(&mut meta);
        assert!(meta.get("flint_kind").is_none(), "flint_kind must be stripped");
        assert_eq!(meta["org"], json!("acme"), "other metadata preserved");
        // A stripped identity classifies as User, not the forged Service.
        let id = Identity {
            session_id: Some("s".into()),
            metadata_public: meta,
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), IdentityKind::User);
    }

    #[test]
    fn strip_untrusted_kind_is_a_noop_on_non_object() {
        let mut null = json!(null);
        strip_untrusted_kind(&mut null); // must not panic
        assert_eq!(null, json!(null));
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

    // ── Delegate classification: well-formed act + spoof-resistance ───────────

    #[test]
    fn well_formed_act_object_classifies_agent_for_any_jwks_idm() {
        // A delegated token from ANY JWKS IdM (gateway-local OR Hydra 8693)
        // carries an `act` actor object + no Kratos session → Agent, gateway-side.
        for act in [
            json!({ "sub": "user-1" }),                       // gateway-local shape
            json!({ "sub": "user-1", "client_id": "hydra" }), // Hydra-delegate shape (act has sub)
        ] {
            let id = Identity {
                metadata_public: json!({ "act": act }),
                ..Default::default()
            };
            assert_eq!(id.derived_kind(), IdentityKind::Agent);
        }
    }

    #[test]
    fn malformed_act_does_not_promote_to_agent() {
        // A non-object / empty `act` is not a valid RFC 8693 actor → safe default.
        for bad in [
            json!(null),
            json!(false),
            json!(""),
            json!("x"),
            json!(42),
            json!([]),
            json!({}), // empty object is not a well-formed actor
        ] {
            let id = Identity {
                metadata_public: json!({ "act": bad }),
                ..Default::default()
            };
            assert_eq!(
                id.derived_kind(),
                IdentityKind::User,
                "act={:?} must NOT promote to Agent",
                id.metadata_public["act"]
            );
        }
    }

    #[test]
    fn act_on_a_kratos_session_does_not_promote() {
        // A `session_id` marks a Kratos identity whose metadata_public may be
        // self-service-writable — an `act` there is not a trustworthy signal.
        let id = Identity {
            session_id: Some("kratos-sess".into()),
            metadata_public: json!({ "act": { "sub": "u" } }),
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), IdentityKind::User);
    }

    #[test]
    fn bare_client_id_never_classifies_non_user() {
        // A normal OIDC access token carries client_id/azp for ordinary users —
        // it must NOT reach an Agent/Service policy (privilege escalation).
        let id = Identity {
            metadata_public: json!({ "client_id": "svc-7", "azp": "svc-7" }),
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), IdentityKind::User);
    }

    #[test]
    fn is_well_formed_act_requires_a_nonempty_sub() {
        assert!(is_well_formed_act(&json!({ "sub": "x" })));
        assert!(is_well_formed_act(&json!({ "sub": "x", "client_id": "c" })));
        // Object without a usable `sub` is NOT a valid actor (RFC 8693 §4.1).
        assert!(!is_well_formed_act(&json!({ "client_id": "c" })));
        assert!(!is_well_formed_act(&json!({ "sub": "" })));
        assert!(!is_well_formed_act(&json!({ "sub": "   " })));
        assert!(!is_well_formed_act(&json!({ "sub": 42 })));
        assert!(!is_well_formed_act(&json!({})));
        assert!(!is_well_formed_act(&json!(null)));
        assert!(!is_well_formed_act(&json!("x")));
        assert!(!is_well_formed_act(&json!([{ "sub": "x" }])));
    }
}
