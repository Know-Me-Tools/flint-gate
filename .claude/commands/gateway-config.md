---
description: Print and validate the active flint-gate config.yaml against the documented schema.
argument-hint: [config-path]   # default: /app/config/config.yaml or ./config.example.yaml
---

Locate and print the active flint-gate configuration, then validate it against the schema described in the `flint-gate-config` skill.

Resolution order for the config path:
1. The `$1` argument, if provided.
2. `FLINT_GATE_CONFIG` env var.
3. `./config.example.yaml` if running inside the repo, else `/app/config/config.yaml`.

```bash
CFG="${1:-${FLINT_GATE_CONFIG:-config.example.yaml}}"
echo "== config path: $CFG =="
```

Then read the file (use the `read` tool, not `cat`) and check, reporting each as PASS/FAIL:

1. **server.listen** present and parses as `host:port`.
2. **server.admin_listen** present and on a different port from `listen`.
3. **database.url** present and starts with `postgres://`.
4. **auth_providers** is a non-empty map; each value has a `type` in `{kratos, jwt, api_key, anonymous}`.
5. Every `auth` reference in `routes` and `sites.default_auth` resolves to a key in `auth_providers`.
6. Every `site` reference in `routes` resolves to an `id` in `sites`.
7. **jwt.signing_algorithm** (if `jwt` block present) is in `{HS256,HS384,HS512,RS256,RS384,RS512,ES256,ES384}`.
8. If `jwt.signing_algorithm` starts with `HS`, flag any non-empty `jwt.signing_key_secret` as a secret-in-config risk and recommend moving it to `FLINT_GATE_JWT_SECRET`.
9. `stream.enabled: true` routes also declare `stream.protocol` (sse) — flag mismatches.
10. No two `routes` share the same `id`.
11. `cache.l2.enabled: true` implies a non-empty `cache.l2.redis_url`.

Output a compact PASS/FAIL table. For each FAIL, cite the file path and the offending field. Do not modify the file; propose the smallest fix and ask the user before applying.
