# Tasks — add-integration-ci-workflow

- [x] Create `.github/workflows/integration.yml` with push/PR triggers on main and active branch
- [x] Add docker build + `docker compose -f docker-compose.test.yml up -d --wait` steps (with failure log dump)
- [x] Add Go integration test step (`go test -v -tags integration -timeout 60s ./...` in `sdks/go/`)
- [x] Add TypeScript integration test step (`pnpm test:integration` in `sdks/typescript/`)
- [x] Add always-run teardown step (`docker compose -f docker-compose.test.yml down -v`); YAML validated
