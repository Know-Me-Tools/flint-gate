# SDKs

Flint Gate provides client SDKs for services that consume its proxy streams or manage it through the admin API.

All SDKs support the proxy data plane (streaming requests) and the admin plane (route/API-key management). Choose the SDK that matches your service language.

| SDK | Package | Repository path |
|-----|---------|-----------------|
| Rust | `flint-gate-client` | `crates/flint-gate-client` |
| Go | `github.com/know-me-tools/flint-gate/sdks/go` | `sdks/go` |
| TypeScript | `@know-me/flint-gate` | `sdks/typescript` |
| Flutter / Dart | `flint_gate` *(coming soon — not yet published)* | `sdks/flutter` |

## Capabilities by SDK

| Capability | Rust | Go | TypeScript | Flutter |
|------------|------|----|------------|---------|
| SSE streaming | Yes | Yes | Yes | Yes |
| WebSocket | Yes | Yes | Yes | No |
| NDJSON | Yes | No | Yes | No |
| Admin API | Yes | Yes | Yes | Partial |
| Identity middleware | No | Yes | Yes | No |

## Quick links

- [Rust SDK](./rust.md)
- [Go SDK](./go.md)
- [TypeScript SDK](./typescript.md)
- [Flutter SDK](./flutter.md)

## Admin API note

The admin API runs on a separate port (`:4457` by default) and must be kept on a private network. SDK admin clients should target an internal address, not the public proxy endpoint.
