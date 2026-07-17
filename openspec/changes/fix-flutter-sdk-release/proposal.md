# fix-flutter-sdk-release

**Phase:** beta-release-readiness / Phase 3 (Medium gap M-2)

## Problem

The `release.yml` workflow publishes the Flutter SDK (`sdks/flutter/`) to
pub.dev as part of every release. The Flutter SDK is a stub — it contains
`pubspec.yaml` and placeholder Dart files but has no working implementation.
Publishing a stub to pub.dev creates a confusing package that beta customers
may discover and attempt to use, only to find it doesn't work.

## Solution

1. Remove the Flutter SDK publish step from `.github/workflows/release.yml`
2. Add a comment in `release.yml` marking where Flutter SDK publishing should
   be added once the implementation is complete
3. Add a "Coming soon" notice to the Flutter SDK `README.md`
4. Add a note to `docs/docs/getting-started.md` under SDK options that the
   Flutter SDK is in development

## Files to change

- `.github/workflows/release.yml` — remove Flutter SDK publish step
- `sdks/flutter/README.md` — add "Coming soon" notice
- `docs/docs/getting-started.md` — note Flutter SDK status
