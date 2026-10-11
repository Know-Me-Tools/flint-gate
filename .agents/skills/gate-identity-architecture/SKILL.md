---
name: gate-identity-architecture
description: "Design Flint Gate identity trust boundaries for JWT/OIDC, Ory Kratos/Hydra, Okta, decentralized identifiers and verifiable credentials."
license: MIT
metadata:
  version: "1.0.0"
  source: "Project-authored from inspected source and primary documentation, 2026-09-25"
---

# gate-identity-architecture

Environment: Flint Gate source checkout; optional Compass MCP or CLI. No external service access is required to read this skill.

Start from auth/identity.rs, jwt_verify.rs, jwt_mint.rs, jwks.rs, kratos.rs, token_exchange.rs and middleware/pipeline.rs under crates/flint-gate-core/src. Inspect the actual credential-to-principal mapping before recommending an adapter.

For each flow specify issuer, audience/resource, human or workload subject, acting agent, tenant, credential type, assurance, expiry, revocation authority and allowed delegation. A verified signature is not authorization. Bind issuer plus subject; do not merge accounts by email alone. Separate authentication, provisioning and authorization.

- JWT: choose an algorithm allowlist independently of untrusted headers; validate signature, issuer, audience, lifetime and purpose. Keep ID tokens, access tokens and internal delegated tokens in distinct validation contexts. Constrain JWKS/issuer URLs against SSRF and resource exhaustion; unknown kid refresh must be bounded. Design overlapping rotation, cache expiry and emergency revocation explicitly.
- Kratos: session/identity management via authoritative session checks is distinct from OAuth authorization-server duties. Assess Hydra or the selected authorization server for OAuth flows. Reject ambiguous credential sources; do not trust federated metadata to mint Gate's agent kind or actor chain.
- Okta: distinguish org authorization-server tokens, intended for Okta, from custom authorization-server tokens for this API. Pin expected issuer and audience, discover/cache keys under that issuer, and account for tenant configuration/licensing before claiming availability. SCIM lifecycle events are not login proofs or immediate token revocation.
- DID/VC: distinguish DID control from verified human identity, credential issuer trust and application privileges. Verify the correct verification relationship, proof suite, challenge/domain binding and presentation replay protections. Select DID methods and credential status mechanisms explicitly; rotation/deactivation is method-dependent. A DID alone conveys no tenant/admin/agent role. Define recovery and privacy/correlation behavior.

Deliver a trust-boundary diagram, claims-to-principal contract, failure matrix and migration ADR with source links. DID/VC and Okta-specific adapters are design capabilities of this role, not asserted implemented Gate features. Changes to auth, policy, proxy or control-plane code go to the respective owner.

## Primary references

- [jwt](https://www.rfc-editor.org/info/rfc8725/)
- [oauth](https://www.rfc-editor.org/info/rfc9700/)
- [did](https://www.w3.org/TR/did/)
- [vc](https://www.w3.org/TR/vc-data-model-2.0/)
- [okta](https://developer.okta.com/docs/guides/validate-access-tokens/dotnet/main/)
- [kratos](https://www.ory.sh/docs/kratos/guides/zero-trust-iap-proxy-identity-access-proxy)
