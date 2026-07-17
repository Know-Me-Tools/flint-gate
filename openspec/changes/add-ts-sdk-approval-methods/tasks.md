# Tasks — add-ts-sdk-approval-methods

- [x] Add `ApprovalStatus` interface and `ApprovalDecision` type to `sdks/typescript/src/types.ts`
- [x] Add `normalizeApproval()` helper (snake_case → camelCase) to `sdks/typescript/src/admin.ts`
- [x] Add `listApprovals(signal?)` → `Promise<ApprovalStatus[]>` to `FlintGateAdmin`
- [x] Add `getApproval(id, signal?)` → `Promise<ApprovalStatus>` to `FlintGateAdmin`
- [x] Add `decideApproval(id, decision, signal?)` → `Promise<void>` to `FlintGateAdmin`
- [x] Verify `pnpm typecheck` and `pnpm test` pass from `sdks/typescript/`
