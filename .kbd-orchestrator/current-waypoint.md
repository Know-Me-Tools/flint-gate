# Current Waypoint

- **Phase:** agent-authz-control-plane
- **Status:** execution_ready
- **Changes:** 0/8 complete
- **Next change:** add-budget-rate-limiting
- **Next command:** `/kbd-apply add-budget-rate-limiting`

## Ordered changes
1. add-budget-rate-limiting (G3) — governor + Redis windows
2. add-mcp-resource-server (G1) — hand-roll RS + jsonwebtoken@9
3. add-policy-engine (G2a) — cedar-policy@4 + arc-swap
4. add-per-tool-authz (G2b) — cedar tool-call gate + list_tools filter
5. add-authz-audit-trail (G6b/G8)
6. add-hitl-approval (G5) — in-band AG-UI/A2UI
7. add-guardrail-hook (G6a) — interface only
8. add-web-config-ui (G4) — rust-embed + React/Vite SPA
