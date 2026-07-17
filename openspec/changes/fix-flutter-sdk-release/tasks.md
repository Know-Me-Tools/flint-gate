- [x] Read `.github/workflows/release.yml` to find Flutter SDK publish step
- [x] Remove the Flutter SDK publish step (or comment it out with a TODO marker)
      Commented out with `# TODO: restore when Flutter SDK implementation is complete.`
- [x] Add `# TODO: restore when Flutter SDK implementation is complete` comment at removal site
- [x] Write `sdks/flutter/README.md` (or update if it exists) with "Coming Soon" notice and link to Go/TS SDKs
- [x] Update `docs/docs/getting-started.md` SDK section to note Flutter SDK is in development (not yet available)
- [x] Verify `release.yml` YAML is still valid after edit — `python3 yaml.safe_load` passes
