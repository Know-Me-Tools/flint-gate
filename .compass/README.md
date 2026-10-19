# Flint Gate code graph

Compass 0.3.28 indexes the Rust crates, operator UI and TypeScript/Go/Flutter SDK sources declared in [config.toml](config.toml). It excludes standalone tests, dependencies, build output and nested worktrees. Inline test modules remain, so statistics are not production-only.

The initial verified snapshot contains **111 files, 5,772 nodes and 12,512 edges**. It includes all three Rust crates, the React console and all three external-language SDK source trees. Source revision and per-file SHA-256 digests, extraction diagnostics, representative call anchors, graph-quality results, store integrity and MCP responses are under `compass-out/verification/`. The graph reflects the working tree, including pre-existing uncommitted source changes, not just HEAD.

From the project root, refresh and verify:

```bash
compass update --inference-level low
node .compass/verify.mjs
```

Use `compass update --help` if the installed CLI changes. Scope lives in config.toml; JSON publication and the SQLite sidecar live in ignored `compass-out/`. No model enrichment or application compilation is needed. The verifier uses Node's filesystem glob APIs; this installation ran Node 26.5.0.

The project [.mcp.json](../.mcp.json) defines `compass-flint-gate` with this command:

```text
/usr/local/bin/compass serve --graph /Users/gqadonis/Projects/prometheus/flint-gate/compass-out/graph.json --transport stdio
```

The connecting client owns process startup/shutdown. No port or background daemon is needed. Reload a client that supports project `.mcp.json`, or explicitly map the same command through that client's supported MCP settings. A successful stdio smoke check proves the server can answer; it does not register it in this existing Codex session or guarantee every client reads `.mcp.json`. After graph refresh, restart/reconnect the client's server to avoid an old in-memory snapshot. Relocating the checkout requires updating the absolute graph path.

The smoke check discovers 18 tools and queries statistics, JWT symbols, JWKS construction and SSE processing calls. It verifies input inventory/digests before and after querying and shuts down its process. Structural edges do not prove runtime reachability or security: macros, dynamic dispatch, SQL/migrations, network/API boundaries and runtime configuration require source and contract inspection. Feature-gated code is not one compiled configuration.

The [agent team](../docs/agent-teams/flint-gate/README.md) uses the graph as navigation evidence. Its architecture decisions remain grounded in current source and executable tests for the selected change.
