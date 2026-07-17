# Tasks — add-go-sdk-policy-methods

- [x] Add `PolicyRow` struct to `sdks/go/types.go`
- [x] Add `PolicyVersionRow`, `PolicyHistoryResponse` structs to `sdks/go/types.go`
- [x] Add `UpsertPolicyInput`, `UpsertPolicyResponse`, `DeletePolicyResponse`, `RollbackPolicyResponse` structs to `sdks/go/types.go`
- [x] Add `ListPolicies` method to `sdks/go/client.go` (unwrap `{"policies":[]}` envelope)
- [x] Add `GetPolicy`, `CreatePolicy`, `UpdatePolicy`, `DeletePolicy` methods to `sdks/go/client.go`
- [x] Add `GetPolicyHistory`, `RollbackPolicy` methods to `sdks/go/client.go`
- [x] Verify `GOROOT=/opt/homebrew/opt/go/libexec go vet -tags integration ./...` clean and `go test ./...` passes
