# Flint Gate team research and architectural assessment

Research date: 2026-09-25. Method: local source/KBD inspection, Compass inventory and call queries, Firecrawl web search/scrape, and Context7 queries for Ory and Okta. [Source receipts](evidence/web-sources.json), [graph evidence](evidence/call-graph.json), [native versions](evidence/native-versions.json) and [skill provenance](evidence/skills-provenance.json) distinguish observations from recommendations. No production identity service was contacted.

## Recommendation

Use ten selectable specialists with an identity/security architect coordinating narrowly scoped changes. Gate combines credential validation, policy enforcement, proxy transformations, long-lived streams and an administrative control plane. A single generic gateway role would hide consequential ownership boundaries; ten simultaneously running agents would add unnecessary coordination cost. Start most security changes with three participants and add specialists only where contracts cross source owners.

The identity architect is explicitly responsible for JWT/OIDC, decentralized identity and verifiable credentials, Ory Kratos/Hydra, Okta, application security and identity lifecycle. This competence is a design/review responsibility. The inspected source does not establish a dedicated Okta adapter or DID/VC verifier, and this setup implements neither. Those features require their own contract, trust model and tests.

## What the code graph establishes

The verified working-tree snapshot indexes 65 Rust, 24 TSX, 11 TypeScript, six Go and five Dart files: 111 total across all three Rust crates, console and SDKs. There are 5,772 nodes and 12,512 edges, no missing/unexpected/stale/failed selected inputs, valid anchors and a valid SQLite store. Inline test modules contribute nodes/edges. Counts are structural, not evidence that every flow works.

| Observed call | Architectural consequence |
|---|---|
| `JwtVerifyAuthenticator::new` → `JwksCache::new` | Token verification and key-cache/rotation design must be reviewed together. |
| Kratos `select_credential` → `extract_cookie` | Session credential ambiguity and authoritative expiry deserve specific tests. |
| Token `exchange` → `build_delegated_claims` | Delegation needs issuer/actor/scope/audience constraints, not merely token syntax. |
| SSE `process_chunk` → `process_line` | Frame boundary correctness is separate from request authentication. |
| `AuthorityConsumer::catch_up` → `replay` | Replicated authorization readiness and replay semantics need an explicit owner. |

These anchors drove the auth, streaming and control-plane assignments. Policy engine/bundle/schema/sugar and approval modules justify a dedicated Cedar owner; middleware and proxy justify a routing owner. The console and four client language surfaces justify separate operator-UX and SDK owners. Operations and independent review complete the pool. Exact byte anchors and source hashes are retained in graph evidence.

Compass cannot resolve every macro, trait dispatch, SQL/migration or network contract. A call edge is not proof of runtime reachability, security or API parity. Review cross-project enforcement using actual source and contract tests.

## Gateway replacement means tested contracts

Oathkeeper documents distinct rule matching, authentication, authorization and mutation stages. Use those stages to define compatibility fixtures for the subset Gate replaces, including failures and identity propagation. Avoid repeating older Gate README claims about what all current Oathkeeper deployments can or cannot do. [Ory pipeline](https://www.ory.sh/docs/oathkeeper/pipeline)

Kong route matching and plugin scoping have their own semantics. Gate should publish a versioned compatibility matrix for host/path/method matching, priority, transforms, identity propagation and errors, with explicit unsupported behavior. A shared label such as “JWT plugin” does not prove equivalent authentication or route selection. [Kong routes](https://developer.konghq.com/gateway/entities/route/), [plugin scope](https://developer.konghq.com/gateway/entities/plugin/)

For Gate's AI-specific surface, add stream expiry/cancellation, tool disclosure before approval, quota reservation/refund and policy-version change fixtures. These are Gate design recommendations based on its source modules; they are not claims of automatic compatibility with either vendor.

## Identity guidance for the architect

Require explicit token-purpose and trust bindings: algorithm, issuer, audience, subject/tenant, lifetime and expected token kind. Review cross-JWT confusion and bounded key retrieval rather than treating successful signature verification as a complete decision. Separate gateway-issued actor/agent authority from untrusted federated claims, consistent with Gate's `identity.rs` and Kratos filtering. [JWT BCP, RFC 8725](https://www.rfc-editor.org/info/rfc8725/)

Review OAuth flows as end-to-end authorization contracts: redirect handling, PKCE, replay and client/token restrictions belong alongside the gateway implementation. Internet drafts updating JWT guidance are research leads, not silently substituted final standards. [OAuth security BCP, RFC 9700](https://www.rfc-editor.org/info/rfc9700/)

Kratos session/identity checking and an OAuth authorization server have different responsibilities. The architect should define how validated Kratos identities become Gate principals and when Hydra or another authorization server issues access tokens. Gate already separates local exchange and Hydra delegation in source; preserve that distinction. [Ory Kratos with an identity-aware proxy](https://www.ory.sh/docs/kratos/guides/zero-trust-iap-proxy-identity-access-proxy)

Okta org-server access tokens are intended for Okta, while a custom authorization server can issue tokens for an application's APIs. Establish the chosen issuer, expected audience, signing-key discovery and validation context before configuring Gate. An ID token is not a substitute API access token; provisioning via SCIM is a separate lifecycle concern. Deployment-specific licensing/configuration remains to be verified when implementing an integration. [Okta access-token validation](https://developer.okta.com/docs/guides/validate-access-tokens/dotnet/main/)

DID control is not proof of human identity or application privilege. Verification relationships and method-specific update/deactivation behavior matter. VC acceptance also depends on trusted issuers, intended proof/presentation context, validity/status and replay controls. Define those contracts before translating a DID or credential into a tenant, admin, human or agent principal. [DID Core](https://www.w3.org/TR/did/), [VC 2.0 trust model](https://www.w3.org/TR/vc-data-model-2.0/#trust-model)

## Authorization, streams and administration

Cedar denies by default and matching forbids override permits; an evaluation error causes that policy to be skipped. Gate must deliberately handle diagnostics, invalid bundles and stale authority at the application boundary. A reviewer should test the combination of a permissive policy and a failing restrictive policy, rather than assume all errors deny. Structured policy/entity construction also avoids turning untrusted input into policy text. [Cedar authorization](https://docs.cedarpolicy.com/auth/authorization.html), [Cedar application security](https://docs.cedarpolicy.com/other/security.html)

Gate-specific recommendations: bind approvals to requester, resource/tool arguments, policy version and expiry; test replay and concurrent consumption. Define stream behavior after headers are sent when authority expires or approval changes. Treat replicated read-model readiness, cursor replay, cache freshness and fencing as authorization inputs. These recommendations follow the local approval, authority, cache and stream module relationships; they are not assertions that every invariant is currently enforced.

Routing hooks, configured identity endpoints and JWKS fetches all introduce outbound URL trust decisions. Use constrained configured destinations, appropriate address/DNS controls and deliberate redirect behavior; retain legitimate private service connectivity without accepting arbitrary client-selected endpoints. [OWASP SSRF prevention](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html)

The admin UI is not an enforcement boundary. Preserve authenticated administration, the private 4457 listener and separate 4456 proxy. SDKs need explicit retry/cancellation/credential propagation semantics. Observability should help diagnose denial/expiry/readiness without publishing raw tokens or tenant data.

## Skills: create focused guidance, preserve legacy material

The four existing `flint-gate-auth/config/deploy/routes` skills were inspected and left intact. Older examples mention limited authentication types, deployment defaults and key-management behavior that need reconciliation with current source and completed phase evidence. They are not bound to the new roles as authoritative setup recipes.

Firecrawl found the [community jwt-security listing](https://www.skills.sh/mindrally/skills/jwt-security), an [Okta SCIM provisioning candidate](https://www.skills.sh/mukul975/anthropic-cybersecurity-skills/implementing-scim-provisioning-with-okta), and an [Okta-to-Descope migration candidate](https://www.skills.sh/descope/skills/okta-cis-to-descope). The first listing was inspected but its complete repository/license was not audited; the latter two address adjacent tasks. Popularity/audit badges and repository names are not endorsements. None was installed.

Instead, ten original MIT-labelled project skills cover delivery, identity architecture, authentication, Cedar/approvals, proxy security, streaming, control plane, clients, operations and independent review. Each has an AgentSkills-compatible name/description/frontmatter, current source anchors, bounded output expectations and primary references where relevant. They contain no executable hooks or downloads. This satisfies the requested “download or create” choice while preserving inspectable provenance and avoiding irrelevant external instructions.

## Native execution and remaining limits

Definitions exist for all requested harness families: Codex, Claude Code, Copilot, Kimi Code, MiniMax CLI, OpenCode and DeepSeek, with UAR and BossFang service artifacts. Native support levels are in the README. Serialization, local installation and an OpenCode discovery check are distinct from model execution or service registration. The local DeepSeek wrapper fails on a package-manager version mismatch; no external installation was altered.

The configured model selections use host-declared availability, explicit strength tiers and unknown prices. They are policy choices, not benchmark or cost guarantees. Per-harness inheritance avoids sending Codex model IDs to unrelated providers. Cross-harness handoffs carry evidence and ownership; shared memory is optional and must have an independently authorized scope.

Current KBD files disagree about the active phase. The workflow explicitly requires reconciliation before subsequent application work and does not advance or rewrite those files. Existing source changes remain untouched. The team is ready as a researched, inspectable set of definitions; feature execution still needs an assigned task and its acceptance criteria.
