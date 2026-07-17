# End-to-end HITL approval flow

This example shows how to configure a streaming route so that a tool call is
paused and held until a human approves or denies it through the Admin API.

## 1. Configure the route

In `config.yaml`, enable the stream and the `authorize` pre-request hook for the
chat route:

```yaml
routes:
  - id: "chat-stream"
    site: "my-app"
    match:
      path: "/api/chat/**"
      methods: ["POST"]
    upstream: "http://llm-backend:8000/v1/chat/completions"
    auth: kratos_session
    priority: 10
    hooks:
      pre_request:
        - type: authorize
          config:
            action: invoke
            enforce: true
    stream:
      enabled: true
      protocol: sse          # or ndjson
      ai:
        ag_ui:
          enabled: true
          validate_events: false
          allowed_events:
            - TEXT_MESSAGE_START
            - TEXT_MESSAGE_CONTENT
            - TEXT_MESSAGE_END
            - TOOL_CALL_START
            - TOOL_CALL_ARGS
            - TOOL_CALL_END
            - RUN_STARTED
            - RUN_FINISHED
            - RUN_ERROR
        a2ui:
          enabled: true
          allowed_intents:
            - render_component
            - show_toast
            - invoke_tool
```

## 2. Load a Cedar policy that requires approval

Use the Admin API to load a policy that promotes any non-empty tool call to a
human approval:

```bash
curl -X PUT http://127.0.0.1:4457/admin/policies/require_approval_for_tools \
  -H "Content-Type: application/json" \
  -d '{
    "policy_text": "@require_approval(\"non-empty tool arguments\")\npermit(principal, action, resource) when { context.arguments != {} };",
    "enabled": true
  }'
```

For A2UI, the `resource` is the intent name (e.g. `invoke_tool`) and
`context.arguments` is the embedded tool call payload.

## 3. Send a streaming request that triggers a tool call

```bash
curl -N -H "Accept: text/event-stream" \
  -H "Cookie: ory_kratos_session=$SESSION" \
  http://127.0.0.1:4456/api/chat \
  -d '{"messages":[{"role":"user","content":"run the dangerous tool"}]}'
```

When the upstream emits a tool call that matches the policy, the stream pauses
and the gateway emits an approval request:

### SSE

```
data: {"type":"GATE_APPROVAL_REQUEST","approvalId":"0190...","principalId":"alice","action":"call_tool","resourceId":"danger","expiresAt":"2026-07-03T12:34:56Z","intent":"non-empty tool arguments"}

```

### NDJSON

```json
{"type":"gate:approval_request","approvalId":"0190...","principalId":"alice","action":"call_tool","resourceId":"danger","expiresAt":"2026-07-03T12:34:56Z","intent":"non-empty tool arguments"}
```

The original `TOOL_CALL_START` / `TOOL_CALL_END` or `invoke_tool` event is held
and not forwarded until a decision is received.

## 4a. Approve → resume

```bash
curl -X POST http://127.0.0.1:4457/admin/approvals/$APPROVAL_ID/decision \
  -H "Content-Type: application/json" \
  -d '{"decision":"approve"}'
```

The gateway releases the held tool-call events into the stream:

```
data: {"type":"TOOL_CALL_START","toolCallId":"c1","toolCallName":"danger"}

data: {"type":"TOOL_CALL_ARGS","toolCallId":"c1","delta":"{\"x\":1}"}

data: {"type":"TOOL_CALL_END","toolCallId":"c1"}

```

## 4b. Deny → abort

```bash
curl -X POST http://127.0.0.1:4457/admin/approvals/$APPROVAL_ID/decision \
  -H "Content-Type: application/json" \
  -d '{"decision":"deny"}'
```

The gateway drops the held event and emits a `RUN_ERROR` into the stream:

```
data: {"type":"RUN_ERROR","error":"tool call denied by operator"}

```

## 5. Expiry

Approval requests expire after 5 minutes by default. An expired request returns
`410 Gone` from the Admin API and is automatically denied in the stream (the
tool call is dropped and a `RUN_ERROR` is emitted).

## Notes

- The Admin API is unauthenticated and must only be reachable inside your
  private network.
- If no `ApprovalManager` is configured, `@require_approval` is treated as a
  deny (fail-closed).
- Multiple pending approvals can exist on the same stream. Each approval id is
  independent and can be resolved in any order.
