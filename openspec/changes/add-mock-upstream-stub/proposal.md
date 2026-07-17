# add-mock-upstream-stub

## Summary

Add a Node.js one-file HTTP stub server that emits a minimal ndjson tool-call stream,
wire it into `docker-compose.test.yml` as a new `mock-upstream` service, and add a
streaming route to `config.test.yaml` pointing at the stub with `stream.enabled: true`
and a Cedar `Authorize` pre-request hook.

This is the prerequisite for the approval full-flow integration tests — without a
streaming upstream that emits a `TOOL_CALL_START` / `TOOL_CALL_ARGS` / `TOOL_CALL_END`
sequence, the stream processor never triggers a per-tool Cedar authorization check.

## Motivation

The `ApprovalManager`, Cedar `@require_approval` annotation, and AG-UI stream processor
approval path are all unit-tested. The only missing piece is an end-to-end integration
path where:
1. A real HTTP request hits the proxy port
2. The stream processor receives a tool-call event from the upstream
3. Cedar evaluates a `@require_approval` policy → returns `RequireApproval`
4. The stream buffers and emits `gate:approval_request`
5. An admin decision unblocks (or discards) the buffered call

The existing fixture `default_upstream: "http://flint-gate:4457/health"` returns
plain JSON — not a streaming ndjson/SSE response — so it cannot trigger the approval
flow regardless of how the route is configured.

## Scope

### Files

| File | Action | Description |
|------|--------|-------------|
| `test/stub/server.mjs` | Create | Node.js HTTP stub — ~30 lines, no dependencies |
| `docker-compose.test.yml` | Edit | Add `mock-upstream` service (node:20-alpine) |
| `config.test.yaml` | Edit | Add `/stream-test` route with stream + Authorize hook |

### Behaviour Spec

**`test/stub/server.mjs`**:
- Listens on port 9999
- `GET /health` → 200 `{"status":"ok"}`
- Any other request → 200 with headers:
  - `Content-Type: application/x-ndjson`
  - `Transfer-Encoding: chunked`
  - `Cache-Control: no-cache`
- Body (3 ndjson lines, each followed by `\n`):
  ```
  {"type":"TOOL_CALL_START","toolCallId":"tc-001","toolCallName":"integ_test_tool"}
  {"type":"TOOL_CALL_ARGS","toolCallId":"tc-001","delta":"{\"x\":1}"}
  {"type":"TOOL_CALL_END","toolCallId":"tc-001"}
  ```

**`docker-compose.test.yml` addition**:
```yaml
mock-upstream:
  image: node:20-alpine
  command: node /stub/server.mjs
  volumes:
    - ./test/stub:/stub:ro
  healthcheck:
    test: ["CMD-SHELL", "wget -qO- http://localhost:9999/health >/dev/null || exit 1"]
    interval: 5s
    timeout: 3s
    retries: 6
    start_period: 5s
```

`flint-gate` depends_on must add:
```yaml
mock-upstream:
  condition: service_healthy
```

**`config.test.yaml` route addition** (under `sites[0]`):
```yaml
routes:
  - id: stream-test
    path: /stream-test
    upstream: "http://mock-upstream:9999"
    stream:
      enabled: true
      protocol: ndjson
    hooks:
      pre_request:
        - type: Authorize
```

## Acceptance Criteria

- [ ] `docker compose -f docker-compose.test.yml up -d --wait` starts all services
      including `mock-upstream` with healthy status
- [ ] `curl -s http://localhost:9999/health` returns `{"status":"ok"}`
- [ ] A POST to `mock-upstream:9999/anything` returns 3 ndjson lines
- [ ] `flint-gate` service still starts and reports healthy after config change
- [ ] No existing integration test broken (the new route is additive)
- [ ] No secrets committed (stub is plain JS, no credentials)

## Security Constraints

- `mock-upstream` is internal to the compose network — not exposed to the host
  (no `ports:` mapping needed; flint-gate reaches it by service name)
- The `/stream-test` route requires a valid JWT on the proxy port (4456) — same
  posture as all other proxied routes
- Cedar policy for `@require_approval` is created at test runtime via Admin API —
  not committed to config files
