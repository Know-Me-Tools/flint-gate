# add-ci-cd

## Summary
Create GitHub Actions CI/CD workflows for the flint-gate monorepo.

## Design
- `.github/workflows/ci.yml`: cargo test/clippy/fmt on every PR
- `.github/workflows/release.yml`: Trusted Publishing skeleton (crates.io OIDC, npm, pub.dev)
- Gate all PRs on green CI

## Tasks
- [ ] Create .github/workflows/ci.yml (cargo test --workspace, clippy, fmt --check)
- [ ] Create .github/workflows/release.yml skeleton (Trusted Publishing for crates.io)
- [ ] Add npm publish job skeleton for sdks/typescript
- [ ] Add pub.dev publish job skeleton for sdks/flutter
- [ ] Add Go module publish job skeleton for sdks/go
