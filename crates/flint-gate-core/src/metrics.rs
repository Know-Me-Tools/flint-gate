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
/// Metric name for delegate-exchange latency (seconds).
pub const DELEGATE_LATENCY: &str = "flint_delegate_latency_seconds";

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
}
