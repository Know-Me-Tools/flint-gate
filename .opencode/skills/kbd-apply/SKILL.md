---
license: MIT
name: kbd-apply
version: '1.0.0'
description: >
  KBD-owned spec-apply driver. Wraps a spec backend (OpenSpec; Spec Kit via the
  speckit adapter) and drives it ONE task at a time, so KBD stays the source of
  truth: every task boundary fires KBD hooks, emits a plain-text position
  signal, and syncs progress.json and the waypoint. Replaces the broken pattern
  of handing the turn to a bare /opsx:apply that runs outside KBD.
metadata:
  tags: [process, orchestration, automation, openspec, spec-kit]
---

# /kbd-apply

The execute-phase work surface. `/kbd-execute` selects the backend and writes
the dispatch contract; **`/kbd-apply` walks the tasks.**

## Why this exists

Previously, after `/kbd-plan` emitted `/opsx:new`, implementation happened by
invoking bare `/opsx:apply` — an unmodified upstream OpenSpec skill that knows
nothing about KBD. It fired no KBD hooks, wrote no `progress.json`, refreshed no
waypoint. The user "got funneled into openspec and lost all connection to the
execute phase." This driver fixes that by making **KBD own the loop** and
calling the spec backend per task.

> **Hard invariant:** never invoke a backend's "do everything" command (bare
> `/opsx:apply`, `/speckit.implement` without a single-task scope). Drive one
> task at a time through this skill.

## The per-task loop (what the model does each turn)

```sh
ROOT="$KBD_ORCHESTRATOR_ROOT"
APPLY="$ROOT/skills/kbd-apply/kbd-apply.sh"
CHANGE="<active change id from the waypoint>"

# 1. Read the task surface (TSV: id \t done \t title)
"$APPLY" list "$CHANGE"
read -r TOTAL COMPLETE REMAINING < <("$APPLY" progress "$CHANGE")

# 2. For each NOT-done task, one per turn:
"$APPLY" begin-task "$CHANGE" "$ID" "$I" "$TOTAL" "$TITLE"
#   → fires task:before, prints "Starting task <I> of <TOTAL>: <TITLE>"

#   <<< implement EXACTLY this one task here (edit code/docs) >>>

"$APPLY" end-task "$CHANGE" "$ID" "$I" "$TOTAL" "$TITLE"
#   → marks the task done in the backend, syncs progress.json + waypoint,
#     fires task:after, prints "Completed task <I> of <TOTAL>: <TITLE>"

# 3. After the LAST task (on_change_complete fired automatically by the
#    index==total sentinel): run the artifact-refiner QA gate, then:
"$APPLY" verify  "$CHANGE"   # backend verify (openspec validate / /opsx:verify)
"$APPLY" archive "$CHANGE"   # backend archive (openspec archive / /opsx:archive)
```

The plain-text "Starting/Completed task i of n" lines are the **user-facing
guarantee** (see `references/per-turn-position-hook.md`); the fired hooks are
the extensibility layer (memory mirror, custom reporters, overrides).

## Subcommands

| Command | Effect |
|---|---|
| `detect [dir]` | print backend id: `openspec`, `speckit`, or empty |
| `list <change>` | tasks as TSV `id⇥done⇥title` |
| `progress <change>` | `total complete remaining` |
| `begin-task <change> <id> <i> <n> <title>` | fire `task:before` + position signal |
| `end-task <change> <id> <i> <n> <title>` | mark done + sync + fire `task:after` + signal |
| `mark-done <change> <id>` | flip one task done (no hooks) |
| `verify <change>` | backend verify; non-zero exit = fail |
| `archive <change>` | backend archive |

## Backends

See `references/spec-backend-interface.md` for the full `SpecBackend` contract
and the verified OpenSpec command/JSON mappings. The OpenSpec adapter is
implemented; the Spec Kit adapter is delivered by change-007.

## Progress Signals (MANDATORY)

```
Starting kbd-apply — <change>
Completed kbd-apply — <change> (<n>/<n> tasks, verified + archived)
```

Per-task `Starting/Completed task <i> of <n>: <title>` signals are emitted by
the driver's `begin-task`/`end-task` — relay them verbatim to the user.

## Relationship to other skills

- `/kbd-execute` — selects backend, writes `execution.md`, then defers task
  execution to this skill.
- `/kbd-reflect` — consumes the per-task `progress.json` this driver maintains.
- Child loops (`/kbd-new-child`, `/kbd-next-child`) use this **same** driver, so
  nested phases get identical per-task reporting (see change-006).
