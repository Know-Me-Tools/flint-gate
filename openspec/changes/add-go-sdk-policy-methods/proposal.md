# add-go-sdk-policy-methods

**Phase:** sdk-integration-test-expansion
**Scope:** `sdks/go/types.go`, `sdks/go/client.go`
**Depends on:** nothing (server endpoints already exist; TypeScript SDK is reference)

## Why

The TypeScript SDK has full Cedar policy CRUD (`listPolicies`, `getPolicy`,
`createPolicy`, `updatePolicy`, `deletePolicy`, `getPolicyHistory`, `rollbackPolicy`)
added in the `add-sdk-policy-methods` change (2026-07-09). The Go SDK has zero
policy types or methods. Operators writing Go integrations cannot manage Cedar
authorization policies programmatically.

## What

### 1. Types (`sdks/go/types.go`)

```go
// PolicyRow is the read shape returned by GET /policies and GET /policies/{id}.
type PolicyRow struct {
    ID           string          `json:"id"`
    PolicyText   string          `json:"policy_text"`
    SchemaJSON   json.RawMessage `json:"schema_json,omitempty"`
    EntitiesJSON json.RawMessage `json:"entities_json,omitempty"`
    Enabled      bool            `json:"enabled"`
    WrittenBy    *string         `json:"written_by,omitempty"`
}

// PolicyVersionRow is one entry in GET /policies/{id}/history.
type PolicyVersionRow struct {
    ID           int             `json:"id"`
    PolicyID     string          `json:"policy_id"`
    VersionNum   int             `json:"version_num"`
    PolicyText   string          `json:"policy_text"`
    SchemaJSON   json.RawMessage `json:"schema_json,omitempty"`
    EntitiesJSON json.RawMessage `json:"entities_json,omitempty"`
    WrittenBy    *string         `json:"written_by,omitempty"`
    WrittenAt    time.Time       `json:"written_at"`
}

// PolicyHistoryResponse is returned by GET /policies/{id}/history.
type PolicyHistoryResponse struct {
    PolicyID  string             `json:"policy_id"`
    TotalHint *int               `json:"total_hint"` // nullable — server omits COUNT(*)
    Versions  []PolicyVersionRow `json:"versions"`
}

// UpsertPolicyInput is the body for POST /policies and PUT /policies/{id}.
type UpsertPolicyInput struct {
    ID           string          `json:"id,omitempty"` // required on POST
    PolicyText   string          `json:"policy_text"`
    SchemaJSON   json.RawMessage `json:"schema_json,omitempty"`
    EntitiesJSON json.RawMessage `json:"entities_json,omitempty"`
    Enabled      bool            `json:"enabled"`
}

// UpsertPolicyResponse is returned by POST /policies and PUT /policies/{id}.
type UpsertPolicyResponse struct {
    Status   string   `json:"status"` // "created" | "updated" | "ok"
    ID       string   `json:"id"`
    Reloaded bool     `json:"reloaded"`
    Warnings []string `json:"warnings,omitempty"`
}

// DeletePolicyResponse is returned by DELETE /policies/{id}.
type DeletePolicyResponse struct {
    Status   string `json:"status"` // "deleted"
    ID       string `json:"id"`
    Reloaded bool   `json:"reloaded"`
}

// RollbackPolicyResponse is returned by POST /policies/{id}/rollback.
type RollbackPolicyResponse struct {
    Status      string   `json:"status"` // "rolled_back" | "ok"
    PolicyID    string   `json:"policy_id"`
    FromVersion int      `json:"from_version"`
    ToVersion   int      `json:"to_version"`
    Reloaded    bool     `json:"reloaded"`
    Warnings    []string `json:"warnings,omitempty"`
}
```

### 2. Client methods (`sdks/go/client.go`)

```go
// ListPolicies calls GET /policies and returns all Cedar authorization policies.
// NOTE: unwraps the {"policies": [...]} envelope the server returns.
func (c *Client) ListPolicies(ctx context.Context) ([]PolicyRow, error)

// GetPolicy calls GET /policies/{id}. Returns an error wrapping 404 when not found.
func (c *Client) GetPolicy(ctx context.Context, id string) (PolicyRow, error)

// CreatePolicy calls POST /policies. The id field in input is required.
func (c *Client) CreatePolicy(ctx context.Context, input UpsertPolicyInput) (UpsertPolicyResponse, error)

// UpdatePolicy calls PUT /policies/{id}.
func (c *Client) UpdatePolicy(ctx context.Context, id string, input UpsertPolicyInput) (UpsertPolicyResponse, error)

// DeletePolicy calls DELETE /policies/{id}. Returns a 404 error if the policy
// does not exist (unlike routes/keys, policy delete is NOT idempotent server-side).
func (c *Client) DeletePolicy(ctx context.Context, id string) (DeletePolicyResponse, error)

// GetPolicyHistory calls GET /policies/{id}/history with optional pagination.
func (c *Client) GetPolicyHistory(ctx context.Context, id string, offset, limit int) (PolicyHistoryResponse, error)

// RollbackPolicy calls POST /policies/{id}/rollback with the target version number.
func (c *Client) RollbackPolicy(ctx context.Context, id string, versionNum int) (RollbackPolicyResponse, error)
```

## Key implementation details

- `ListPolicies` MUST unwrap `{"policies": [...]}` envelope (same as `ListApprovals`)
- `json.RawMessage` for `SchemaJSON`/`EntitiesJSON` — avoids double-encoding while preserving the raw JSON blob
- `PolicyVersionRow.WrittenAt` is RFC3339 from the server; `time.Time` handles this natively via `json.Unmarshal`
- Policy delete is NOT server-side idempotent (404 on missing) — do NOT swallow 404 like `DeleteRoute`

## Verification

- `GOROOT=/opt/homebrew/opt/go/libexec go vet -tags integration ./...` clean from `sdks/go/`
- `GOROOT=/opt/homebrew/opt/go/libexec go test ./...` (unit tests) still pass
