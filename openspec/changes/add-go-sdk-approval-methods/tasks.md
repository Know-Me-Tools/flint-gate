# Tasks — add-go-sdk-approval-methods

- [x] Add `ApprovalStatus` struct and `ApprovalDecision` string type + constants to `sdks/go/types.go`
- [x] Add `ListApprovals(ctx)` → `([]ApprovalStatus, error)` to `sdks/go/client.go`
- [x] Add `GetApproval(ctx, id)` → `(ApprovalStatus, error)` to `sdks/go/client.go`
- [x] Add `DecideApproval(ctx, id, decision)` → `error` to `sdks/go/client.go`
- [x] Verified `go vet .` clean and `go test ./...` passes from `sdks/go/`
