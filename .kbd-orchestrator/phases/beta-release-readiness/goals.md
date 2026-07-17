# Goals — beta-release-readiness

_Requested by: user (2026-07-09)_

Determine whether flint-gate's current feature set is ready for release to
external beta customers who will use it for real work in real deployments.

## Goals

1. **Gap identification** — enumerate every gap between the current codebase
   and what a production deployment requires, with honest severity ratings.

2. **Blocker vs. acceptable risk classification** — distinguish gaps that
   would cause data loss, security incidents, or silent correctness failures
   from gaps that are tolerable in a beta.

3. **Closure map** — for each blocker, identify what specific work closes it
   and an estimated change count.
