---
license: MIT
name: kbd-status
version: '1.0.0'
description: >
  Show current KBD process status — active phase, change inventory, goal
  completion, and next recommended action. Reads progress.json to surface
  work completed by all tools (Antigravity, Roo, Cursor, Cline, Codex, etc.)
metadata:
  tags: [process, orchestration, automation]
---

# /kbd-status

Show current KBD process state for the active project.

## What this does

Reads orchestrator state and progress ledger, then produces a complete status
summary including cross-tool work visibility.

Output includes:

- Project name and active phase
- OpenSpec changes: active / in-progress / archived (if OpenSpec available)
- Native KBD changes: status from `progress.json`
- Goal completion: MET | PARTIAL | NOT MET per goal
- Last tool to update state and when
- Waypoint-guided next recommended action

## Progress Signals (MANDATORY)

When the status output is ready to display, emit:

```
Completed kbd-status — <phase-name>
```

The status output itself serves as the start signal. Emit the completion signal after printing the full status table. Use the canonical phase name from `current-waypoint.json`. Emit to plain response text — no tool call needed.

## How to invoke

1. **Discover project identity** — read `.kbd-orchestrator/project.json` or infer
2. **Read waypoint** — `.kbd-orchestrator/current-waypoint.json`
3. **Read progress** — `.kbd-orchestrator/phases/<phase>/progress.json`
4. **Resolve worktree root** — see *Worktree awareness* below
5. **Render phase chain** — see *Phase chain rendering* below
6. **Read phase artifacts** — `assessment.md`, `plan.md`, `execution.md`
7. **If OpenSpec**: read `openspec/changes/` active + `openspec/changes/archive/`
8. **Print status table**

## Position model (preferred source)

When `.kbd-orchestrator/position.json` exists and is not older than
`current-waypoint.json`, render from it FIRST — it is the unified, derived
position tree (`shared/lib/position.sh` → `kbd_position_sync`; schema
`references/schemas/position.schema.json`):

- `phase:` line = `cursor` joined with the chain separator
  (e.g. `phase: parent › child › change-007 › task:4/8`).
- One indented line per tree level with its `progress.done/total`.
- `annotations[]` render one line each after the tree:
  `note (evolver): evolutions: my-evolution` — foreign state surfaced
  read-only, never modified.

Fall back to waypoint-based rendering below when position.json is absent,
invalid, or staler than the waypoint. Refresh it any time by sourcing
`shared/lib/position.sh` and calling `kbd_position_sync`.

## Phase chain rendering

The active phase is rendered as a chain that reflects the nested-phase fields
documented in `kbd-process-orchestrator/SKILL.md` → "Nested phases":

- `parentPhase = null` and `childPhases = []` → `phase: <name>` (unchanged from pre-schema behavior).
- `parentPhase = null` and `childPointer = <c>` selecting a member of `childPhases` → `phase: <parent> › <c>` and a follow-up line `children: <i>/<n>` (1-based index of the pointer + total count).
- `parentPhase = <p>` (this row is itself a child) → `phase: <p> › <name>`. If the row *also* has a `childPointer`, append `› <pointer>` for grand-child chains.
- `childPhases` non-empty but `childPointer = null` → `phase: <name>  (children defined, none active)`.

The separator character is U+203A `›` by default. When `LC_ALL` or `LANG` is
`POSIX` / `C` / `C.*`, the skill falls back to ` > ` (space-greater-space).

The chain is sourced from `shared/lib/waypoint.sh`: call `waypoint_load` to
parse the waypoint with documented defaults, then `waypoint_chain` to render.

## Worktree awareness

After rendering the phase line, the skill renders one mandatory `worktree:`
line, immediately before the `change:` line. Resolution order:

1. Resolve `worktreeRoot` from `project.json` (`worktreeRoot` field). If
   absent or unreadable, fall back to the literal string `${HOME}/.claude/worktrees`.
2. Expand `${HOME}` (and `${USER}` for parity) in the resolved value using the
   current environment. The helper `expand_kbd_path` in
   `shared/lib/waypoint.sh` does this safely (no `eval`).
3. Compute `git rev-parse --show-toplevel` from the current working directory.

Then emit:

- Inside the root: `worktree: <path>` (no annotation).
- Outside the root, or exactly at the root itself: `worktree: <path>  ⚠ outside worktreeRoot (<resolved-root>)`.
- Not inside a git checkout: `worktree: (none — not inside a git checkout)`.
- `git` not on `PATH`: `worktree: (none — git not available)`.
- `project.json` unreadable (permissions, corrupt JSON): `worktree: <path>  ⚠ project.json unreadable, using default root`.

The skill never blocks on any of these conditions — the rest of the status
report MUST always render.

## Render order (append-only stability)

To preserve grep-based scripts and human muscle memory, lines emitted in
`Output Example` retain their existing positions. The two blocks introduced
by `ssed-kbd-nested-phase-schema` slot in here:

1. `phase: …` *(existing)*
2. `children: i/n` *(new — only when childPhases populated and childPointer set, or `(children defined, none active)` when set without pointer)*
3. `worktree: …` *(new — always rendered)*
4. `Last updated by: …` *(existing — and every line below)*

## Output Example

```
KBD STATUS — <Project Name>
phase: phase-2-sales-module
worktree: /Users/jane/.claude/worktrees/phase-2-sales-module
Last updated by: roo-code (2026-03-12T04:30:00Z)

Goals:
  [✅] Sidebar user profile wired to real data
  [🔄] Team invitations email flow (IN_PROGRESS — started by antigravity)
  [⬜] Clients page full implementation
  [⬜] Deals 4-step wizard

Changes:
  DONE:        change-006-sidebar-user-profile (completed by: antigravity)
  IN_PROGRESS: change-007-complete-team-invitations (4/8 tasks, started by: antigravity)
  PENDING:     change-008-clients-page
  PENDING:     change-009-deals-page

Next action (from waypoint): /kbd-apply change-007-complete-team-invitations
```

### Example — nested phase, outside worktreeRoot

```
KBD STATUS — <Project Name>
phase: submodule-skills-and-entity-devtools-expansion › ssed-kbd-process-hooks
children: 3/11
worktree: /Users/jane/projects/foo  ⚠ outside worktreeRoot (/Users/jane/.claude/worktrees)
Last updated by: claude-code (2026-05-27T00:00:00Z)
...
```

## Dual-audience output (`--explain` + ux_profile)

`kbd-status` serves both advanced and beginner readers from one source:

- **Default (dense)** — the status table plus the *header lines* of the active
  phase's `decision-log.md` (one line per decision: `D-001 · <decision>
  [stage · date]`). Advanced users scan and move on.
- **`--explain`** — expands each decision-log header into its full
  TL;DR / Why / Alternatives / Learn-more block, and appends a "what happens
  next and why" narrative derived from the waypoint's `exactNextCommand` and
  the current stage. Beginners see *what* was decided, *why*, and *what to
  learn*.

The default verbosity is set by `ux_profile` in `project.json`
(`"beginner"` → `--explain` on by default; `"advanced"` → dense). `ux_profile`
NEVER gates information — it only changes ordering/expansion; everything is
always reachable with or without the flag.

Decision-log entries follow `references/templates/decision-log.template.md` and
are written by kbd-analyze, kbd-plan, pmpo-elicit, and pmpo-outer-loop.

## Examples

```
/kbd-status                   # current project + active phase (dense, or per ux_profile)
/kbd-status phase-1-foundation # status of a specific phase
/kbd-status --explain          # expand decision-log entries + next-and-why narrative
```
