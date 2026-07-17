# Tasks — fix-ts-listpolicies-envelope

- [x] Fix `listPolicies` in `sdks/typescript/src/admin.ts` to unwrap `{"policies":[]}` envelope (also updated unit test mock to match real server shape)
- [x] Verify `pnpm typecheck` clean and `pnpm test` passes (16 unit tests pass, 4 integration skipped)
