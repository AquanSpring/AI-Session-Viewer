use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use session_core::parser::path_encoder::get_projects_dir;
use session_core::provider::{claude, codex, grok, omp};
use session_core::watcher_batch::{collect_until_quiet, is_session_file_change};

/// Minimum interval between emitting fs-change events to the frontend.
const DEBOUNCE_DURATION: Duration = Duration::from_millis(300);
const MAX_BATCH_DURATION: Duration = Duration::from_secs(1);
const WATCH_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Start watching Claude, Codex, Grok and OMP session directories for changes.
/// Emits "fs-change" events to the frontend when files are modified.
/// Events are debounced to avoid flooding the frontend during batch operations.
pub fn start_watcher(app_handle: AppHandle) -> Result<(), String> {
    let claude_dir = get_projects_dir();
    let codex_dir = codex::get_sessions_dir();
    let grok_dir = grok::get_sessions_dir();
    let omp_dir = omp::get_sessions_dir();

    std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

        let mut watcher = match RecommendedWatcher::new(tx, Config::default()) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Failed to create watcher: {}", e);
                return;
            }
        };

        let directories = [&claude_dir, &codex_dir, &grok_dir, &omp_dir];
        let mut watched = [false; 4];

        loop {
            // A provider may create its session root after the application
            // starts. Retry missing roots periodically instead of giving up
            // at startup when the user has no sessions yet.
            sync_watches(&mut watcher, &directories, &mut watched);

            let first = match rx.recv_timeout(WATCH_REFRESH_INTERVAL) {
                Ok(event) => event,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let events =
                collect_until_quiet(first, DEBOUNCE_DURATION, MAX_BATCH_DURATION, |timeout| {
                    rx.recv_timeout(timeout)
                });
            let mut changed = HashSet::new();
            for event in events {
                match event {
                    Ok(event) => {
                        changed.extend(
                            event
                                .paths
                                .into_iter()
                                .filter(|path| is_session_file_change(path)),
                        );
                    }
                    Err(error) => eprintln!("Watch error: {error}"),
                }
            }

            if !changed.is_empty() {
                let paths: Vec<PathBuf> = changed.into_iter().collect();
                let is_claude_change = claude_dir
                    .as_ref()
                    .map(|dir| paths.iter().any(|path| path.starts_with(dir)))
                    .unwrap_or(false);
                let is_codex_change = codex_dir
                    .as_ref()
                    .map(|dir| paths.iter().any(|path| path.starts_with(dir)))
                    .unwrap_or(false);
                let is_grok_change = grok_dir
                    .as_ref()
                    .map(|dir| paths.iter().any(|path| path.starts_with(dir)))
                    .unwrap_or(false);
                let is_omp_change = omp_dir
                    .as_ref()
                    .map(|dir| paths.iter().any(|path| path.starts_with(dir)))
                    .unwrap_or(false);

                // Hand each provider only the paths under its own
                // directory, so it can surgically update just the
                // affected projects/files instead of wiping everything.
                if is_claude_change {
                    if let Some(ref dir) = claude_dir {
                        let provider_paths: Vec<PathBuf> = paths
                            .iter()
                            .filter(|p| p.starts_with(dir))
                            .cloned()
                            .collect();
                        if !provider_paths.is_empty() {
                            claude::invalidate_paths(&provider_paths);
                        }
                    }
                }
                if is_codex_change {
                    if let Some(ref dir) = codex_dir {
                        let provider_paths: Vec<PathBuf> = paths
                            .iter()
                            .filter(|p| p.starts_with(dir))
                            .cloned()
                            .collect();
                        if !provider_paths.is_empty() {
                            codex::invalidate_paths(&provider_paths);
                        }
                    }
                }
                if is_grok_change {
                    if let Some(ref dir) = grok_dir {
                        let provider_paths: Vec<PathBuf> = paths
                            .iter()
                            .filter(|p| p.starts_with(dir))
                            .cloned()
                            .collect();
                        if !provider_paths.is_empty() {
                            grok::invalidate_paths(&provider_paths);
                        }
                    }
                }
                if is_omp_change {
                    if let Some(ref dir) = omp_dir {
                        let provider_paths: Vec<PathBuf> = paths
                            .iter()
                            .filter(|path| path.starts_with(dir))
                            .cloned()
                            .collect();
                        if !provider_paths.is_empty() {
                            omp::invalidate_paths(&provider_paths);
                        }
                    }
                }

                let paths: Vec<String> = paths
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();

                let _ = app_handle.emit("fs-change", paths);
            }
        }
    });

    Ok(())
}

fn sync_watches(
    watcher: &mut RecommendedWatcher,
    directories: &[&Option<PathBuf>; 4],
    watched: &mut [bool; 4],
) {
    for (index, directory) in directories.iter().enumerate() {
        let Some(directory) = directory else {
            continue;
        };
        if !directory.exists() {
            if watched[index] {
                let _ = watcher.unwatch(directory);
                watched[index] = false;
            }
        } else if !watched[index] {
            match watcher.watch(directory, RecursiveMode::Recursive) {
                Ok(()) => watched[index] = true,
                Err(error) => eprintln!("Failed to watch {:?}: {}", directory, error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn watches_provider_root_created_after_startup() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "ai-session-viewer-watch-late-root-{}-{unique}",
            std::process::id()
        ));
        let directories = [Some(directory.clone()), None, None, None];
        let mut watched = [false; 4];
        let (tx, _rx) = mpsc::channel::<notify::Result<Event>>();
        let mut watcher = RecommendedWatcher::new(tx, Config::default()).unwrap();

        sync_watches(&mut watcher, &directories.each_ref(), &mut watched);
        assert!(!watched[0]);

        fs::create_dir_all(&directory).unwrap();
        sync_watches(&mut watcher, &directories.each_ref(), &mut watched);
        assert!(watched[0]);

        drop(watcher);
        fs::remove_dir_all(directory).unwrap();
    }
}
