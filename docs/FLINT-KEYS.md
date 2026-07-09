# Flint Key Contract

`flint-gate` is the production signing and verification boundary for Flint
project keys.

## Accepted Key Shapes

- `FLINT_ANON_KEY`: publishable JWT/API key, maps to role `anon`.
- `FLINT_SERVICE_ROLE_KEY`: server-only key, maps to role `service_role`.
- Future opaque keys use `flint_pk_...` and `flint_sk_...` prefixes.

API-key rows preserve the role and principal type:

| Column | Example | Meaning |
|---|---|---|
| `role` | `anon`, `agent`, `service_role` | Downstream Postgres/API role |
| `principal_type` | `User`, `Agent`, `Service` | Cedar principal family |
| `scopes` | `["read:documents"]` | OAuth-style scope set |

`flint_sk_...` keys are rejected when presented by browser-like user agents.
Secret keys must only be used by trusted server-side workloads.

## Trusted Headers

Before forwarding to `flint-forge` or `flint-realtime-fabric`, Gate strips
client-supplied `X-Flint-*` headers and injects gateway-owned headers:

- `X-Flint-User-Id`
- `X-Flint-Role`
- `X-Flint-Principal-Type`
- `X-Flint-Tenant-Id`
- `X-Flint-Verified-By: flint-gate`
- `X-Flint-JTI`
- `X-Flint-Agent-Id`
- `X-Flint-Workflow-Id`
- `X-Flint-Scope`

Downstream services should trust these headers only when
`X-Flint-Verified-By` is exactly `flint-gate`.
