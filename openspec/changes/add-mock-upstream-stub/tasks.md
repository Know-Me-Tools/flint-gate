# Tasks — add-mock-upstream-stub

- [ ] Create `test/stub/` directory and write `test/stub/server.mjs` (~30 lines Node.js HTTP server)
- [ ] Add `mock-upstream` service to `docker-compose.test.yml` (node:20-alpine, healthcheck)
- [ ] Add `mock-upstream: service_healthy` to `flint-gate` depends_on in `docker-compose.test.yml`
- [ ] Add `/stream-test` route to `config.test.yaml` (upstream mock-upstream:9999, stream.enabled, protocol ndjson, Authorize hook)
- [ ] Verify `docker compose -f docker-compose.test.yml config` parses without errors
