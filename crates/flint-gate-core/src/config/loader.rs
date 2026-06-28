/// YAML configuration loader with filesystem hot-reload.
///
/// Provides:
/// - Initial load from a YAML file at startup
/// - `notify`-based file watcher with debouncing
/// - A `tokio::sync::watch` channel so subscribers receive updated configs
use crate::config::types::GateConfig;
use anyhow::{Context, Result};
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{watch, RwLock};
use tracing::{error, info, warn};

/// Shared, mutable reference to the current gate configuration.
pub type SharedConfig = Arc<RwLock<GateConfig>>;

/// Load the YAML config file and return a shared config + watch receiver.
///
/// The watch receiver fires whenever the config file changes and is
/// successfully re-parsed.
pub async fn load_config(
    path: impl AsRef<Path>,
) -> Result<(SharedConfig, watch::Receiver<GateConfig>)> {
    let path = path.as_ref().to_path_buf();
    let initial = parse_yaml(&path).await?;

    info!(path = %path.display(), "loaded initial config");

    let shared = Arc::new(RwLock::new(initial.clone()));
    let (tx, rx) = watch::channel(initial);

    // Spawn the file watcher in the background
    let shared_clone = Arc::clone(&shared);
    let path_clone = path.clone();
    tokio::spawn(async move {
        if let Err(e) = watch_file(path_clone, shared_clone, tx).await {
            error!(error = %e, "config file watcher exited with error");
        }
    });

    Ok((shared, rx))
}

/// Parse YAML from the given path into a [`GateConfig`].
async fn parse_yaml(path: &Path) -> Result<GateConfig> {
    let content = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading config file {}", path.display()))?;
    let cfg: GateConfig = serde_yaml::from_str(&content).with_context(|| "parsing config YAML")?;
    Ok(cfg)
}

/// Watch the config file and send updates through `tx` on changes.
async fn watch_file(
    path: PathBuf,
    shared: SharedConfig,
    tx: watch::Sender<GateConfig>,
) -> Result<()> {
    let (event_tx, mut event_rx) =
        tokio::sync::mpsc::unbounded_channel::<notify::Result<notify::Event>>();

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = event_tx.send(res);
        },
        NotifyConfig::default(),
    )
    .context("creating file watcher")?;

    watcher
        .watch(&path, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", path.display()))?;

    // Debounce: wait at least 200 ms after last event before reloading.
    const DEBOUNCE: Duration = Duration::from_millis(200);
    let mut last_event = Instant::now() - DEBOUNCE * 2;

    loop {
        match event_rx.recv().await {
            None => {
                warn!("config watcher channel closed");
                break;
            }
            Some(Err(e)) => {
                warn!(error = %e, "file watch error");
            }
            Some(Ok(_event)) => {
                let now = Instant::now();
                if now.duration_since(last_event) < DEBOUNCE {
                    // Skip — within debounce window
                    continue;
                }
                last_event = now;

                // Small sleep to let the write complete
                tokio::time::sleep(Duration::from_millis(50)).await;

                match parse_yaml(&path).await {
                    Ok(new_cfg) => {
                        info!(path = %path.display(), "config reloaded");
                        *shared.write().await = new_cfg.clone();
                        let _ = tx.send(new_cfg);
                    }
                    Err(e) => {
                        error!(error = %e, "failed to parse updated config; keeping previous");
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn load_minimal_config() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "server:\n  listen: \"0.0.0.0:9999\"").unwrap();
        let (shared, _rx): (SharedConfig, _) = load_config(f.path()).await.unwrap();
        let cfg = shared.read().await;
        assert_eq!(cfg.server.listen, "0.0.0.0:9999");
    }

    #[tokio::test]
    async fn load_empty_config_uses_defaults() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "{{}}").unwrap();
        let (shared, _rx): (SharedConfig, _) = load_config(f.path()).await.unwrap();
        let cfg = shared.read().await;
        assert_eq!(cfg.server.listen, "0.0.0.0:4456");
    }
}
