# add-go-sdk-approval-methods

**Phase:** agent-authz-budget-rate-limiting
**Scope:** `sdks/go/types.go`, `sdks/go/client.go`
**Depends on:** nothing (server endpoints already exist)

## Why

The admin API exposes three approval endpoints (`GET /approvals`, `GET /approvals/{id}`,
`POST /approvals/{id}/decision`) and these are fully implemented in the Rust core.
The Go SDK has no corresponding client methods. An operator writing a Go integration
cannot list, inspect, or decide on pending human-in-the-loop approvals programmatically.

## What

### 1. Types (`sdks/go/types.go`)

Add `ApprovalStatus` struct mirroring the server's serialized shape:

```go
type ApprovalStatus struct {
    ApprovalID  string    `json:"approval_id"`
    PrincipalID string    `json:"principal_id"`
    Action      string    `json:"action"`
    ResourceID  string    `json:"resource_id"`
    Reason      *string   `json:"reason,omitempty"`
    ExpiresAt   time.Time `json:"expires_at"`
    Expired     bool      `json:"expired"`
}
```

Add `ApprovalDecision` string type with typed constants:

```go
type ApprovalDecision string

const (
    ApprovalDecisionApprove ApprovalDecision = "approve"
    ApprovalDecisionDeny    ApprovalDecision = "deny"
)
```

Note: server uses `#[serde(rename_all = "snake_case")]` → JSON values are `"approve"` and `"deny"`.

### 2. Client methods (`sdks/go/client.go`)

```go
// ListApprovals returns all non-expired pending approvals.
func (c *Client) ListApprovals(ctx context.Context) ([]ApprovalStatus, error)

// GetApproval returns the status of a single pending approval by id.
// Returns an error wrapping 404 when the approval is not found or already resolved.
func (c *Client) GetApproval(ctx context.Context, id string) (ApprovalStatus, error)

// DecideApproval approves or denies a pending approval.
// Returns an error wrapping 404 (not found) or 410 (expired).
func (c *Client) DecideApproval(ctx context.Context, id string, decision ApprovalDecision) error
```

## Verification

- `go vet -tags integration .` clean from `sdks/go/`
- `go test ./...` (unit tests) still pass
- Manual shape verification against `crates/flint-gate-core/src/approval/mod.rs` `ApprovalStatus` struct
