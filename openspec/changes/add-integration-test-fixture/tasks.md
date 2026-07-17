# Tasks — add-integration-test-fixture

- [x] Create `docker-compose.test.yml` (based on `docker-compose.smoke.yml`, drop `web`, use loopback admin_listen)
- [x] Create `config.test.yaml` with loopback admin_listen (AllowLoopback posture, no auth required)
- [x] Verify compose file structure mirrors proven smoke stack; CI step validates live startup
