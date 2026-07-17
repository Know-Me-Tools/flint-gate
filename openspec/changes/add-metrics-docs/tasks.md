# Tasks — add-metrics-docs

- [x] Write `docs/docs/metrics.md` with all 6 metric entries (name, type, labels, description, alert threshold)
- [x] Write `grafana/flint-gate-dashboard.json` with panels for each metric
- [x] Update `docs/sidebars.ts` — add `'metrics'` under Operations category
- [x] Update `docs/docs/operations.md` — add `GET /metrics` endpoint reference and link to metrics.md
- [x] Verify sidebar structure with `node -e` (structural check, no pnpm install needed)
