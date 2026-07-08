//! Prometheus metrics surface for the gateway control plane.
//!
//! Installs a global [`metrics`] recorder backed by
//! [`metrics_exporter_prometheus`] and exposes a render handle. Instrumentation
//! call sites use the lightweight `metrics::counter!` / `metrics::histogram!`
//! macros (decoupled from the exporter). The rendered text is served on the
//! **admin** port (private control plane) — never the public proxy surface.
//!
//! The recorder is process-global and install-once; a second install is a
//! no-op (the existing handle is reused), so tests and multiple call sites are
//! safe.

use std::sync::OnceLock;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

/// Metric name for delegate-exchange outcomes (labelled `result`).
pub const DELEGATE_TOTAL: &str = "flint_delegate_total";
/// Metric name for gateway-**local** token-exchange (mint) outcomes (labelled
/// `result`). Symmetric with [`DELEGATE_TOTAL`]: together they make both
/// RFC 8693 exchange modes — Hydra-delegate and local-mint — observable.
pub const LOCAL_EXCHANGE_TOTAL: &str = "flint_local_exchange_total";
/// Metric name for delegate-exchange latency (seconds).
pub const DELEGATE_LATENCY: &str = "flint_delegate_latency_seconds";
/// Metric name for per-tool-call authz decisions (labelled `decision`).
pub const TOOL_AUTHZ_TOTAL: &str = "flint_tool_authz_total";
/// Metric name for over-budget denials of agent-scoped budgets.
pub const AGENT_BUDGET_DENIED_TOTAL: &str = "flint_agent_budget_denied_total";
/// Metric name for route hot-reloads rejected by the strict agent-governance lint
/// (under-governed route in the reloaded set → reload rejected, last-good retained).
pub const GOVERNANCE_RELOAD_REJECTED_TOTAL: &str = "flint_governance_reload_rejected_total";

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Install the global Prometheus recorder (idempotent) and return a render
/// handle. Safe to call more than once — the first install wins and subsequent
/// calls return a clone of the same handle.
pub fn install_recorder() -> PrometheusHandle {
    HANDLE
        .get_or_init(|| {
            PrometheusBuilder::new()
                .install_recorder()
                .expect("install Prometheus recorder")
        })
        .clone()
}

/// Render the current metrics in Prometheus text format. Returns an empty
/// string when the recorder was never installed (metrics disabled).
pub fn render() -> String {
    HANDLE.get().map(PrometheusHandle::render).unwrap_or_default()
}

/// Record one delegate-exchange outcome: increments
/// `flint_delegate_total{result="<reason>"}`. `reason` is a stable, low-
/// cardinality label (e.g. `success`, `deny_transport`, `deny_non2xx`).
pub fn record_delegate(reason: &'static str) {
    metrics::counter!(DELEGATE_TOTAL, "result" => reason).increment(1);
}

/// Record delegate-exchange latency in seconds under `flint_delegate_latency_seconds`.
pub fn record_delegate_latency(seconds: f64) {
    metrics::histogram!(DELEGATE_LATENCY).record(seconds);
}

/// Record one gateway-**local** token-exchange (mint) outcome: increments
/// `flint_local_exchange_total{result="<reason>"}`. `reason` is a stable, low-
/// cardinality **`&'static str`** label (e.g. `success`, `deny_verify`,
/// `deny_downscope`, `mint_failed`) — the symmetric counterpart to
/// [`record_delegate`] so both exchange modes are observable.
pub fn record_local_exchange(reason: &'static str) {
    metrics::counter!(LOCAL_EXCHANGE_TOTAL, "result" => reason).increment(1);
}

/// Record one per-tool-call authz decision: increments
/// `flint_tool_authz_total{decision="<decision>"}`. `decision` is a stable,
/// low-cardinality **`&'static str`** label (e.g. `allow`, `deny`,
/// `deny_shadow`) — the tool NAME is deliberately NOT a label (it is
/// operator/attacker-influenced and would explode cardinality; it stays in the
/// DB authz audit trail).
pub fn record_tool_authz(decision: &'static str) {
    metrics::counter!(TOOL_AUTHZ_TOTAL, "decision" => decision).increment(1);
}

/// Record one over-budget denial of an **agent**-scoped budget: increments
/// `flint_agent_budget_denied_total`. Surfaces the volume of agent spend caps
/// actually enforced (over-limit or fail-closed on a backend outage).
pub fn record_agent_budget_denied() {
    metrics::counter!(AGENT_BUDGET_DENIED_TOTAL).increment(1);
}

/// Record one route hot-reload rejected by strict agent-governance: increments
/// `flint_governance_reload_rejected_total`. Makes a rejected (retain-last-good)
/// reload observable, not just log-grep-able — an operator can alert on it.
pub fn record_governance_reload_rejected() {
    metrics::counter!(GOVERNANCE_RELOAD_REJECTED_TOTAL).increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_idempotent_and_render_reflects_a_counter() {
        let _h = install_recorder();
        // A second install must not panic.
        let _h2 = install_recorder();
        record_delegate("success");
        let out = render();
        // The rendered text names the metric once a value exists.
        assert!(out.contains("flint_delegate_total"), "render:\n{out}");
        assert!(out.contains("result=\"success\""), "render:\n{out}");
    }

    #[test]
    fn render_exposes_every_delegate_reason_and_latency() {
        // The recorder is process-global; assert PRESENCE of each label (not an
        // exact count) so this is robust under parallel test execution.
        install_recorder();
        for reason in [
            "success",
            "deny_transport",
            "deny_non2xx",
            "deny_badjson",
            "deny_actor_token",
        ] {
            record_delegate(reason);
        }
        record_delegate_latency(0.012);
        let out = render();
        for reason in [
            "deny_transport",
            "deny_non2xx",
            "deny_badjson",
            "deny_actor_token",
        ] {
            assert!(
                out.contains(&format!("result=\"{reason}\"")),
                "missing result={reason} in render:\n{out}"
            );
        }
        // Latency histogram is exposed under its metric name.
        assert!(
            out.contains("flint_delegate_latency_seconds"),
            "missing latency histogram in render:\n{out}"
        );
    }

    #[test]
    fn render_exposes_every_local_exchange_reason() {
        // Process-global recorder: assert PRESENCE of each label, not an exact
        // count, so this is robust under parallel execution.
        install_recorder();
        for reason in ["success", "deny_verify", "deny_downscope", "mint_failed"] {
            record_local_exchange(reason);
        }
        let out = render();
        assert!(
            out.contains("flint_local_exchange_total"),
            "missing local-exchange metric in render:\n{out}"
        );
        for reason in ["success", "deny_verify", "deny_downscope", "mint_failed"] {
            assert!(
                out.contains(&format!("result=\"{reason}\"")),
                "missing result={reason} in render:\n{out}"
            );
        }
    }

    #[test]
    fn render_exposes_tool_authz_and_budget_denied() {
        install_recorder();
        record_tool_authz("allow");
        record_tool_authz("deny");
        record_agent_budget_denied();
        let out = render();
        assert!(out.contains("flint_tool_authz_total"), "render:\n{out}");
        assert!(out.contains("decision=\"allow\""), "render:\n{out}");
        assert!(out.contains("decision=\"deny\""), "render:\n{out}");
        assert!(
            out.contains("flint_agent_budget_denied_total"),
            "render:\n{out}"
        );
    }

    #[test]
    fn render_exposes_governance_reload_rejected() {
        install_recorder();
        record_governance_reload_rejected();
        let out = render();
        assert!(
            out.contains("flint_governance_reload_rejected_total"),
            "render:\n{out}"
        );
    }
}
