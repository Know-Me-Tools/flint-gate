/// YAML configuration loader with filesystem change detection.
///
/// Provides:
/// - Initial load from a YAML file at startup
/// - `notify`-based file watcher with debouncing
/// - A `tokio::sync::watch` channel so subscribers can require a restart
use crate::config::types::GateConfig;
use anyhow::{Context, Result};
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, RwLock};
use tracing::{error, info, warn};

/// Shared startup configuration. File changes never replace this value while
/// the process is running; callers may use the watch receiver to report that a
/// restart is required.
pub type SharedConfig = Arc<RwLock<GateConfig>>;

/// Load the YAML config file and return a shared config + watch receiver.
///
/// The watch receiver advances whenever a change to the configured path is
/// observed. The active configuration remains the startup snapshot even when
/// the changed file is malformed; subscribers use the signal to require a
/// restart.
pub async fn load_config(path: impl AsRef<Path>) -> Result<(SharedConfig, watch::Receiver<u64>)> {
    let path = if path.as_ref().is_absolute() {
        path.as_ref().to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolving current directory for config file")?
            .join(path.as_ref())
    };
    let initial = parse_yaml(&path).await?;

    info!(path = %path.display(), "loaded initial config");

    let shared = Arc::new(RwLock::new(initial.clone()));
    let (tx, rx) = watch::channel(0_u64);

    // Spawn the file watcher in the background
    let path_clone = path.clone();
    tokio::spawn(async move {
        if let Err(e) = watch_file(path_clone, tx).await {
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

/// Watch the original configured path and report changes after a trailing-edge
/// debounce. Parsing is intentionally limited to startup: a malformed update
/// still requires a restart, and no file event mutates the active snapshot.
async fn watch_file(path: PathBuf, tx: watch::Sender<u64>) -> Result<()> {
    let (event_tx, mut event_rx) =
        tokio::sync::mpsc::unbounded_channel::<notify::Result<notify::Event>>();

    let configured_event_tx = event_tx.clone();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = configured_event_tx.send(res);
        },
        NotifyConfig::default(),
    )
    .context("creating file watcher")?;

    let watch_directory = path.parent().unwrap_or_else(|| Path::new("."));
    watcher
        .watch(watch_directory, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", watch_directory.display()))?;

    let mut observed_target = std::fs::canonicalize(&path).ok();
    let mut target_watch = None;
    update_target_watch(
        &event_tx,
        watch_directory,
        &mut target_watch,
        observed_target.as_deref(),
    );

    // Debounce on the trailing edge so a partial-write burst cannot consume the
    // only notification before the final filesystem event arrives.
    const DEBOUNCE: Duration = Duration::from_millis(200);
    let mut pending_deadline = None;

    loop {
        if let Some(deadline) = pending_deadline {
            tokio::select! {
                event = event_rx.recv() => {
                    match event {
                        None => {
                            warn!("config watcher channel closed");
                            break;
                        }
                        Some(Err(e)) => warn!(error = %e, "file watch error"),
                        Some(Ok(event)) => {
                            if event_matches(&event, &path, &mut observed_target) {
                                update_target_watch(
                                    &event_tx,
                                    watch_directory,
                                    &mut target_watch,
                                    observed_target.as_deref(),
                                );
                                pending_deadline = Some(tokio::time::Instant::now() + DEBOUNCE);
                            }
                        }
                    }
                }
                () = tokio::time::sleep_until(deadline) => {
                    pending_deadline = None;
                    let revision = tx.borrow().wrapping_add(1);
                    if tx.send(revision).is_err() {
                        break;
                    }
                    warn!(path = %path.display(), "configuration file changed; restart required");
                }
            }
        } else {
            match event_rx.recv().await {
                None => {
                    warn!("config watcher channel closed");
                    break;
                }
                Some(Err(e)) => warn!(error = %e, "file watch error"),
                Some(Ok(event)) => {
                    if event_matches(&event, &path, &mut observed_target) {
                        update_target_watch(
                            &event_tx,
                            watch_directory,
                            &mut target_watch,
                            observed_target.as_deref(),
                        );
                        pending_deadline = Some(tokio::time::Instant::now() + DEBOUNCE);
                    }
                }
            }
        }
    }

    Ok(())
}

fn update_target_watch(
    event_tx: &tokio::sync::mpsc::UnboundedSender<notify::Result<notify::Event>>,
    configured_directory: &Path,
    target_watch: &mut Option<(PathBuf, RecommendedWatcher)>,
    resolved_target: Option<&Path>,
) {
    let next_directory = resolved_target
        .and_then(Path::parent)
        .filter(|directory| *directory != configured_directory)
        .map(Path::to_path_buf);
    if target_watch.as_ref().map(|(directory, _)| directory) == next_directory.as_ref() {
        return;
    }

    if let Some((previous, mut previous_watcher)) = target_watch.take() {
        if let Err(error) = previous_watcher.unwatch(&previous) {
            warn!(%error, path = %previous.display(), "could not release previous config target watch");
        }
    }

    if let Some(next) = next_directory {
        let target_event_tx = event_tx.clone();
        match RecommendedWatcher::new(
            move |result| {
                let _ = target_event_tx.send(result);
            },
            NotifyConfig::default(),
        ) {
            Ok(mut target_watcher) => {
                match target_watcher.watch(&next, RecursiveMode::NonRecursive) {
                    Ok(()) => *target_watch = Some((next, target_watcher)),
                    Err(error) => {
                        warn!(%error, path = %next.display(), "could not watch resolved config target")
                    }
                }
            }
            Err(error) => {
                warn!(%error, path = %next.display(), "could not create config target watcher")
            }
        }
    }
}

fn event_matches(
    event: &notify::Event,
    configured_path: &Path,
    observed_target: &mut Option<PathBuf>,
) -> bool {
    let current_target = std::fs::canonicalize(configured_path).ok();
    let target_changed = current_target != *observed_target;
    let exact_path_changed = event.paths.iter().any(|changed| {
        changed == configured_path
            || observed_target
                .as_deref()
                .is_some_and(|target| changed == target)
            || current_target
                .as_deref()
                .is_some_and(|target| changed == target)
    });
    *observed_target = current_target;
    exact_path_changed || target_changed
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
