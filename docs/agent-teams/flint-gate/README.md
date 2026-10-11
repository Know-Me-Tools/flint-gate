# Flint Gate development team

This is a project-local specialist pool for Flint Gate's identity-aware gateway, with an explicit domain expert for JWTs, decentralized identity, Ory Kratos/Hydra, Okta and application security. Ten roles and ten original AgentSkills-format skills are installed as definitions; no agents, workflows or remote registrations were started.

Read the [research and design rationale](RESEARCH.md), [task selection and handoff workflow](WORKFLOW.md), [canonical manifest](team-request.json), [skill provenance](evidence/skills-provenance.json) and [verification receipt](evidence/validation.json). The guide's general-purpose proposal is in [evidence/guide.json](evidence/guide.json); the manifest refines it using Gate's actual source boundaries.

| Role | Responsibility | Specialist skills, in addition to gate-team-delivery |
|---|---|---|
| gate-identity-architect | Identity/security architecture and team lead; JWT/OIDC, DID/VC, Kratos/Hydra, Okta | gate-identity-architecture |
| gate-auth | Authenticators, key handling, token minting/exchange and provider integration | gate-auth-implementation, gate-identity-architecture |
| gate-policy | Cedar schemas/evaluation, approvals and guardrails | gate-cedar-approvals |
| gate-proxy | Routing, middleware, HTTP boundaries and gateway composition | gate-proxy-security |
| gate-streams | SSE/NDJSON/WebSocket, AG-UI/A2UI, rate and budget enforcement | gate-streaming-governance, gate-cedar-approvals |
| gate-control-plane | Admin API, configuration, database/cache and replicated authority | gate-control-plane |
| gate-console | Operator UI, accessibility and accurate security states | gate-client-security |
| gate-sdk | Rust, TypeScript, Go and Flutter contracts | gate-client-security |
| gate-operations | Deployment, CI, metrics and recovery | gate-operations |
| gate-reviewer | Independent security and compatibility evidence | gate-security-review, gate-identity-architecture |

The manifest assigns disjoint write paths. Roles may read broadly. The default for a security-sensitive change is architect, one relevant implementer and independent reviewer. An isolated UI/documentation change may need only one implementer. Choose the needed roles per task rather than launching the entire roster. Four concurrent roles and one Rust build owner are coordination defaults, not enforced runtime settings.

## Harness definitions

| Harness | Project location | Verification/application level |
|---|---|---|
| Codex | `.codex/agents/gate-*.toml` | Installed project definitions; format inspected, no agent invocation |
| Claude Code | `.claude/agents/gate-*.md` | Installed project subagents; alternative plugin/marketplace staged only |
| Copilot | `.github/agents/gate-*.agent.md` | Installed custom-agent definitions; source/format verification |
| Kimi Code | `.kimi-code/agents/gate-*.md` | Installed project definitions; plugin/v2 marketplace staged only |
| OpenCode | `.opencode/agents/gate-*.md` | Installed; native `agent list --pure` discovers all ten |
| MiniMax CLI | `.agent-team/minimax/agents/<role>/agent.md` | Installed in isolated project data profile; no runtime acceptance claimed |
| DeepSeek Harness | `exports/deepseek/.dsh/profiles/` | Staged; local wrapper fails pnpm version check, runtime unverified |
| UAR | `exports/uar/uar/agents/` | Registration artifacts staged; instance/tool policy selection still required |
| BossFang | `exports/bossfang/bossfang/` | Standalone-agent/workflow route selected as future deployment path; no registration/activation |

Canonical skills are in `.agents/skills/gate-*/SKILL.md`, with project discovery links for Codex, Claude, Copilot, Kimi and OpenCode. Prompts also explicitly read these files as a fallback. Existing skills, marketplace and harness settings were preserved. The optional Claude/Kimi plugin copies are alternatives to the installed project agents, not a second installation to enable simultaneously. Distributing a plugin later also requires packaging its bound skills.

Use `node docs/agent-teams/flint-gate/install.mjs` to restore missing project definitions or the ignored MiniMax profile. It preflights collisions, preserves existing different files and rejects symlinked destination ancestors. To launch MiniMax with this local profile from the project root:

```bash
MINIMAX_DATA_DIR="$PWD/.agent-team/minimax" mcode
```

The isolated profile may need its own operator-configured credentials. No credentials were copied. Custom-agent selection must use the installed MiniMax interface; no unsupported `mcode exec --agent` flag is assumed. DeepSeek profiles and its separate experimental team-composition profile are proposals; they do not create team members. The external dsh installation was left unchanged because this task is Gate-only.

## Model policy

The [catalog](models.json) uses host-declared availability with explicit unknown prices. `agent-team-models` selected `gpt-6-astra`/high for architecture, auth, policy, proxy, streams, control plane and review; `gpt-5.6-sol`/medium for routine console, SDK and operations work. Concrete Codex bindings live in each role's native override and selection receipts. Escalate identity or security-sensitive work performed by a medium-tier role to the hard policy before invocation.

OpenCode uses the configured `kimi-for-coding/k3`; Claude inherits its caller's model. Kimi uses its configured invocation/default model (`kimi-code/k3` observed on this host), not an invented per-role frontmatter field. Other targets inherit their configured provider/model where their contract allows. Codex IDs are not exported as cross-provider model promises. No provider inference was run and no monetary ceiling or cheapest-model claim is made. Low-tier work needs explicit model discovery first. Refresh discovery, capability evidence and pricing through agent-team-models/liter-llm before making budget guarantees.

## Graph and validation

See [.compass/README.md](../../../.compass/README.md). The verified graph has 111 files, 5,772 nodes and 12,512 edges. Run these from the project root:

```bash
node .compass/verify.mjs
node docs/agent-teams/flint-gate/verify.mjs
```

The first validates graph freshness, inventory, quality, SQLite and stdio MCP. The second validates ownership, skill bindings, native export/install hashes, model selections and formats. Setup preserved the 19 pre-existing modified source files; no Rust/SDK/UI application code or KBD state was changed. Generated graphs/runtime state are ignored. No commits, pushes, service deployments or global MCP/harness registrations were performed.
