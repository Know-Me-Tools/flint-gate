/// Universal identity representation returned by all authenticators.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A successfully authenticated principal.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Identity {
    /// Unique subject identifier (user ID, service account ID, etc.)
    pub id: String,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
