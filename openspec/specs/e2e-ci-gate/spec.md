# e2e-ci-gate Specification

## Purpose
TBD - created by archiving change add-e2e-ci-workflow. Update Purpose after archive.
## Requirements
### Requirement: A GitHub Actions workflow SHALL run all Playwright smoke tests on every push and pull request targeting main or the active feature branch

`.github/workflows/e2e-smoke.yml` MUST define a job that installs dependencies, installs
Chromium via `playwright install chromium --with-deps`, and runs `pnpm test:e2e --project chromium`
with `CI: true` set in the environment. The job MUST run on `push` and `pull_request` events
targeting `main` and `claude/flint-gate-auth-proxy-zQBD4`.

#### Scenario: Workflow triggers on push to main

Given a commit is pushed to the `main` branch,
when GitHub Actions evaluates workflow triggers,
then the `e2e / Playwright smoke tests` job SHALL be queued.

#### Scenario: Workflow triggers on pull request to main

Given a pull request targets the `main` branch,
when GitHub Actions evaluates workflow triggers,
then the `e2e / Playwright smoke tests` job SHALL be queued.

### Requirement: The CI job SHALL upload the Playwright HTML report as a GitHub Actions artifact when any test fails

The upload step MUST use `if: failure()` so it only runs when the job exits non-zero.
The artifact MUST be named `playwright-report`, retain for 7 days, and include the `web/playwright-report/` directory.

#### Scenario: Report artifact is uploaded on test failure

Given the E2E job completes with at least one failing test,
when GitHub Actions evaluates the upload step,
then a `playwright-report` artifact SHALL be published with a 7-day retention period.

#### Scenario: Report artifact is NOT uploaded on all-pass run

Given the E2E job completes with zero failing tests,
when GitHub Actions evaluates the upload step,
then no artifact SHALL be uploaded (the step SHALL be skipped).

### Requirement: Playwright smoke tests that require a live admin server SHALL be skipped automatically when running in CI

`web/e2e/smoke.spec.ts` tests that call the real admin API MUST call `test.skip()` when
`process.env.CI` is truthy, so the CI job does not fail due to the absence of a running
admin server on port 4457.

#### Scenario: Live-server tests are skipped in CI

Given `CI=true` is set in the environment,
when the Playwright test runner starts,
then any test guarded by `skipWithoutServer()` SHALL have status "skipped" in the report.

#### Scenario: Live-server tests run locally without CI flag

Given `CI` is not set or is falsy in the environment,
when the Playwright test runner starts,
then tests guarded by `skipWithoutServer()` SHALL execute normally (pass or fail based on outcome).

