//! Watcher thread: notify + debounce, send path list to main.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

/// Run the watcher: spawn a thread that watches `root` and sends debounced absolute paths
/// over `tx`. Main should receive from the corresponding `rx` and call `handle_file_change` for each path.
/// The thread exits when `tx` is dropped (receiver disconnected) or on watcher error.
pub fn run_watcher_thread(
    root: &Path,
    debounce_ms: u64,
    tx: mpsc::Sender<std::path::PathBuf>,
) -> crate::error::Result<()> {
    let root = root.to_path_buf();
    let debounce = Duration::from_millis(debounce_ms);

    let (event_tx, event_rx) = mpsc::channel::<Vec<std::path::PathBuf>>();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            let _ = event_tx.send(ev.paths);
        }
    })
    .map_err(|e| crate::error::RagmcpError::Config(e.to_string()))?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| crate::error::RagmcpError::Config(e.to_string()))?;

    let mut pending: HashMap<std::path::PathBuf, Instant> = HashMap::new();

    loop {
        match event_rx.recv_timeout(debounce) {
            Ok(paths) => {
                let now = Instant::now();
                for p in paths {
                    pending.insert(p, now);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                let ready: Vec<_> = pending
                    .iter()
                    .filter(|(_, t)| now.duration_since(**t) >= debounce)
                    .map(|(p, _)| p.clone())
                    .collect();
                for p in &ready {
                    pending.remove(p);
                }
                for p in ready {
                    if tx.send(p).is_err() {
                        return Ok(());
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}
