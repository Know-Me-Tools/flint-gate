/// WebSocket upstream bridge — connects to an upstream WS endpoint and
/// pipes frames bidirectionally, applying AG-UI/A2UI/Cedar filtering on text frames.
///
/// Uses `tokio-tungstenite` for the upstream connection (already in the
/// dependency tree via axum's `ws` feature) and axum's built-in WS for
/// the client-facing side.
///
/// When `tool_authz` is `Some`, Cedar policy is evaluated on every
/// `TOOL_CALL_START` frame received from the upstream (same semantics as
/// the SSE/NDJSON processors). Tool calls denied by Cedar are dropped and
/// a `RUN_ERROR` frame is sent to the client instead.
///
/// When `approval_handle` is `Some`, `RequireApproval` Cedar decisions
/// cause the WS bridge to register the pending tool call with the
/// `ApprovalManager` and await a human decision before forwarding. This
/// requires the caller to also process `approval_rx` (the decision channel)
/// and call `AgUiProcessor::resolve_approval` to unblock the held frame.
use crate::approval::ApprovalManager;
use crate::config::types::StreamConfig;
use crate::stream::a2ui::{A2UiEvent, A2UiProcessor};
use crate::stream::ag_ui::{AgUiEvent, AgUiProcessor, AgUiTokenCounter};
use crate::stream::{ApprovalHandleParts, StreamMetrics};
use axum::extract::ws::{Message, WebSocket};
use futures::SinkExt;
use futures::StreamExt;
use std::time::Instant;

#[derive(Clone)]
struct WsBridgeApprovalHandle {
    manager: ApprovalManager,
    decision_tx: tokio::sync::mpsc::UnboundedSender<(String, crate::approval::ApprovalDecision)>,
    ttl_override: Option<std::time::Duration>,
}

/// Bridge a client WebSocket to an upstream WebSocket endpoint.
///
/// 1. Connects to `upstream_url` via `tokio_tungstenite::connect_async`.
/// 2. Pipes frames bidirectionally.
/// 3. On upstream→client text frames, applies Cedar/AG-UI/A2UI filtering.
/// 4. Enforces backpressure (duration + event count).
///
/// `tool_authz` — when `Some`, Cedar policy is evaluated per tool call.
/// `approval_handle` — when `Some`, `RequireApproval` Cedar decisions pause
/// the stream until a human decision arrives via the shared `ApprovalManager`.
pub async fn ws_bridge(
    client_socket: WebSocket,
    upstream_url: &str,
    config: &StreamConfig,
    user_scopes: Vec<String>,
    metadata: serde_json::Map<String, serde_json::Value>,
    theme: Option<serde_json::Value>,
    tool_authz: Option<crate::authz::ToolAuthzContext>,
    approval_handle: Option<ApprovalHandleParts>,
) -> StreamMetrics {
    let metrics = StreamMetrics::default();
    let started_at = Instant::now();

    // Connect to upstream WebSocket
    let upstream_result = tokio_tungstenite::connect_async(upstream_url).await;
    let (upstream, _) = match upstream_result {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!(error = %e, upstream = %upstream_url, "failed to connect to upstream WS");
            return metrics;
        }
    };

    tracing::info!(upstream = %upstream_url, "WS bridge established");

    // Split both connections into sender/receiver
    let (mut client_tx, mut client_rx) = client_socket.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();

    // Wire approval handle parts into a typed struct for the AG-UI processor.
    let approval_handle = approval_handle.map(|(manager, decision_tx, ttl_override)| {
        WsBridgeApprovalHandle {
            manager,
            decision_tx,
            ttl_override,
        }
    });

    // Set up AG-UI/A2UI processors, threading Cedar authz and approval routing.
    let ag_ui_processor = if config.ai.ag_ui.enabled {
        let max_tool_args_bytes = config
            .ai
            .backpressure
            .max_tool_args_bytes
            .unwrap_or(crate::stream::DEFAULT_MAX_TOOL_ARGS_BYTES);
        let mut proc = AgUiProcessor::new(
            config.ai.ag_ui.validate_events,
            config.ai.ag_ui.allowed_events.clone(),
        )
        .with_tool_authz(tool_authz)
        .with_max_tool_args_bytes(max_tool_args_bytes);
        if let Some(ref handle) = approval_handle {
            proc = proc.with_approval_handle(
                handle.manager.clone(),
                handle.decision_tx.clone(),
                handle.ttl_override,
            );
        }
        Some(proc)
    } else {
        None
    };

    let a2ui_processor = if config.ai.a2ui.enabled {
        Some(A2UiProcessor::new(config.ai.a2ui.allowed_intents.clone()))
    } else {
        None
    };

    let token_counter = AgUiTokenCounter::default();
    let metrics_total = std::sync::Arc::new(tokio::sync::Mutex::new(metrics));
    let token_counter = std::sync::Arc::new(tokio::sync::Mutex::new(token_counter));

    // Upstream → client (with AG-UI/A2UI filtering)
    let ag_ui = ag_ui_processor.clone();
    let a2ui = a2ui_processor.clone();
    let scopes = user_scopes.clone();
    let meta = metadata.clone();
    let theme_val = theme.clone();
    let metrics_clone = Arc::clone(&metrics_total);
    let tc_clone = Arc::clone(&token_counter);
    let max_secs = config.ai.backpressure.max_stream_duration_seconds;
    let max_events = config.ai.backpressure.max_events;
    let started = started_at;

    let upstream_to_client = async move {
        while let Some(msg_result) = upstream_rx.next().await {
            // Backpressure checks
            if let Some(max_s) = max_secs {
                if started.elapsed().as_secs() > max_s {
                    let _ = client_tx
                        .send(Message::Text(
                            "{\"type\":\"RUN_ERROR\",\"message\":\"stream limit exceeded\"}".into(),
                        ))
                        .await;
                    let _ = client_tx.send(Message::Close(None)).await;
                    break;
                }
            }

            {
                let m = metrics_clone.lock().await;
                if let Some(max_ev) = max_events {
                    if m.total_events >= max_ev {
                        let _ = client_tx
                            .send(Message::Text(
                                "{\"type\":\"RUN_ERROR\",\"message\":\"stream limit exceeded\"}"
                                    .into(),
                            ))
                            .await;
                        let _ = client_tx.send(Message::Close(None)).await;
                        break;
                    }
                }
            }

            match msg_result {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    let text_str = text.as_str();
                    let mut metrics_guard = metrics_clone.lock().await;
                    metrics_guard.total_events += 1;
                    drop(metrics_guard);

                    let filtered = filter_ws_text(
                        text_str, &ag_ui, &a2ui, &scopes, &meta, &theme_val, &tc_clone,
                    )
                    .await;

                    match filtered {
                        Some(json) => {
                            let mut metrics_guard = metrics_clone.lock().await;
                            metrics_guard.passed_events += 1;
                            drop(metrics_guard);
                            if client_tx.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                        None => {
                            let mut metrics_guard = metrics_clone.lock().await;
                            metrics_guard.dropped_events += 1;
                            drop(metrics_guard);
                        }
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Binary(data)) => {
                    if client_tx.send(Message::Binary(data)).await.is_err() {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                    let _ = client_tx.send(Message::Close(None)).await;
                    break;
                }
                Ok(tokio_tungstenite::tungstenite::Message::Ping(data)) => {
                    let _ = client_tx.send(Message::Pong(data)).await;
                }
                Ok(tokio_tungstenite::tungstenite::Message::Pong(_)) => {}
                Ok(tokio_tungstenite::tungstenite::Message::Frame(_)) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "upstream WS error");
                    break;
                }
            }
        }
    };

    // Client → upstream (passthrough)
    let client_to_upstream = async move {
        while let Some(msg_result) = client_rx.next().await {
            match msg_result {
                Ok(Message::Text(text)) => {
                    let text_str = text.to_string();
                    if upstream_tx
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            text_str.into(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Binary(data)) => {
                    if upstream_tx
                        .send(tokio_tungstenite::tungstenite::Message::Binary(data))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Close(_)) => {
                    let _ = upstream_tx
                        .send(tokio_tungstenite::tungstenite::Message::Close(None))
                        .await;
                    break;
                }
                Ok(Message::Ping(data)) => {
                    let _ = upstream_tx
                        .send(tokio_tungstenite::tungstenite::Message::Ping(data))
                        .await;
                }
                Ok(Message::Pong(_)) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "client WS error");
                    break;
                }
            }
        }
    };

    tokio::select! {
        _ = upstream_to_client => {},
        _ = client_to_upstream => {},
    }

    // Finalize metrics
    let mut m = metrics_total.lock().await;
    m.duration_ms = started_at.elapsed().as_millis() as u64;
    {
        let tc = token_counter.lock().await;
        m.estimated_tokens = tc.estimated_tokens();
    }
    m.clone()
}

/// Filter a WS text frame through AG-UI/A2UI processors.
async fn filter_ws_text(
    text: &str,
    ag_ui: &Option<AgUiProcessor>,
    a2ui: &Option<A2UiProcessor>,
    scopes: &[String],
    metadata: &serde_json::Map<String, serde_json::Value>,
    theme: &Option<serde_json::Value>,
    token_counter: &Arc<tokio::sync::Mutex<AgUiTokenCounter>>,
) -> Option<String> {
    // Try AG-UI processing. `process_multi` returns 0..N events: 0 when dropped
    // or HELD (buffered tool call), N when a `TOOL_CALL_END` releases a held
    // call. Multiple released events are newline-joined into one WS text frame.
    if let Some(proc) = ag_ui {
        if let Some(event) = AgUiEvent::from_json(text) {
            {
                let mut tc = token_counter.lock().await;
                tc.count_event(&event);
            }
            let released = proc.process_multi(event, metadata.clone());
            if released.is_empty() {
                return None;
            }
            let joined = released
                .iter()
                .map(AgUiEvent::to_json)
                .collect::<Vec<_>>()
                .join("\n");
            return Some(joined);
        }
    }

    // Try A2UI processing
    if let Some(proc) = a2ui {
        if let Some(event) = A2UiEvent::from_json(text) {
            match proc.process(event, scopes, theme.clone()) {
                Some(processed) => return Some(processed.to_json()),
                None => return None,
            }
        }
    }

    // No AI processing — pass through
    Some(text.to_string())
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authz::{AuthzEngine, PolicyRecord, PrincipalKind, ToolAuthzContext};

    fn tool_authz_ctx(policy: &str) -> ToolAuthzContext {
        let engine = AuthzEngine::from_records(&[PolicyRecord {
            id: "test".to_string(),
            policy_text: policy.to_string(),
            schema_json: None,
            entities_json: None,
        }])
        .expect("valid policy");
        ToolAuthzContext {
            engine: Arc::new(engine),
            principal_kind: PrincipalKind::User,
            principal_id: "test-user".to_string(),
            route_id: "ws-route".to_string(),
            revoked: false,
            audit: None,
        }
    }

    /// Verify that the ws_bridge AgUiProcessor construction path accepts None
    /// for both new params without panicking (backward-compatible call site).
    #[test]
    fn ws_bridge_none_tool_authz_builds_without_panic() {
        let proc = AgUiProcessor::new(false, vec![])
            .with_tool_authz(None::<ToolAuthzContext>)
            .with_max_tool_args_bytes(1024 * 1024);
        drop(proc);
    }

    /// Verify that a Cedar deny-all policy, threaded via the same path as
    /// ws_bridge, replaces the TOOL_CALL_START with a RUN_ERROR (not forwarded).
    #[test]
    fn ws_bridge_cedar_deny_emits_run_error_for_tool_call() {
        let ctx = tool_authz_ctx("forbid(principal, action, resource);");
        let proc = AgUiProcessor::new(false, vec![])
            .with_tool_authz(Some(ctx))
            .with_max_tool_args_bytes(1024 * 1024);

        let start_json = r#"{"type":"TOOL_CALL_START","toolCallId":"tc1","toolCallName":"read_file"}"#;
        let event = AgUiEvent::from_json(start_json).expect("valid event");
        let released = proc.process_multi(event, serde_json::Map::new());
        // Cedar deny → START is replaced by a RUN_ERROR (not forwarded as-is)
        assert!(
            !released.is_empty(),
            "Cedar deny should produce a RUN_ERROR frame, not silence"
        );
        assert_eq!(
            released[0].event_type, "RUN_ERROR",
            "Cedar deny should produce RUN_ERROR, got {:?}",
            released[0].event_type
        );
    }

    /// Verify that a Cedar permit-all policy allows tool call frames through.
    #[test]
    fn ws_bridge_cedar_permit_releases_tool_call_on_end() {
        let ctx = tool_authz_ctx("permit(principal, action, resource);");
        let proc = AgUiProcessor::new(false, vec![])
            .with_tool_authz(Some(ctx))
            .with_max_tool_args_bytes(1024 * 1024);

        // START is buffered — not released until END is seen
        let start_json = r#"{"type":"TOOL_CALL_START","toolCallId":"tc1","toolCallName":"read_file"}"#;
        let start_ev = AgUiEvent::from_json(start_json).expect("valid");
        let held = proc.process_multi(start_ev, serde_json::Map::new());
        assert!(held.is_empty(), "START frame is held pending END");

        // END releases the buffered START + END pair
        let end_json = r#"{"type":"TOOL_CALL_END","toolCallId":"tc1"}"#;
        let end_ev = AgUiEvent::from_json(end_json).expect("valid");
        let released = proc.process_multi(end_ev, serde_json::Map::new());
        assert!(!released.is_empty(), "Cedar permit should release buffered events on END");
    }

    /// Verify that the ws_bridge approval-handle wiring path compiles with a
    /// real ApprovalManager (construction-level coverage for task 3).
    #[test]
    fn ws_bridge_approval_handle_wires_into_ag_ui() {
        use crate::approval::ApprovalManager;
        let manager = ApprovalManager::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = tool_authz_ctx("permit(principal, action, resource);");
        let proc = AgUiProcessor::new(false, vec![])
            .with_tool_authz(Some(ctx))
            .with_max_tool_args_bytes(1024 * 1024)
            .with_approval_handle(manager, tx, None);
        drop(proc);
    }
}
