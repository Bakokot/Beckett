//! Watches the project folder for changes made outside the IDE (the Godot
//! editor, git, other tools) and reports the touched paths in batches.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, StreamExt};
use iced::Subscription;
use notify::{EventKind, RecursiveMode, Watcher};

pub fn watch(root: PathBuf) -> Subscription<Vec<PathBuf>> {
    Subscription::run_with(root, |root| {
        let root = root.clone();
        iced::stream::channel(16, async move |mut output| {
            let (tx, mut rx) = mpsc::unbounded();
            let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                let Ok(event) = result else {
                    return;
                };
                if matches!(event.kind, EventKind::Access(_)) {
                    return;
                }
                for path in event.paths.into_iter().filter(|p| !ignored(p)) {
                    let _ = tx.unbounded_send(path);
                }
            });
            // Bound for the whole block, so it watches as long as the
            // subscription runs.
            let Ok(mut watcher) = watcher else {
                return std::future::pending::<()>().await;
            };
            if watcher.watch(&root, RecursiveMode::Recursive).is_err() {
                return std::future::pending::<()>().await;
            }
            while let Some(first) = rx.next().await {
                // One save fires several events; report each path once.
                let mut paths = BTreeSet::from([first]);
                while let Ok(path) = rx.try_recv() {
                    paths.insert(path);
                }
                if output.send(paths.into_iter().collect()).await.is_err() {
                    return;
                }
            }
        })
    })
}

/// Godot's caches and temporary save files change constantly; skip them.
fn ignored(path: &Path) -> bool {
    let in_cache = path.components().any(|c| {
        matches!(c, Component::Normal(name) if name == ".godot" || name == ".git" || name == ".import")
    });
    let temporary = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".tmp") || n.ends_with('~'));
    in_cache || temporary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_caches_and_temp_files() {
        assert!(ignored(Path::new("/game/.godot/editor/state.cfg")));
        assert!(ignored(Path::new("/game/scenes/player.gd.tmp")));
        assert!(!ignored(Path::new("/game/scenes/player.gd")));
        assert!(!ignored(Path::new("/game/my.godot.notes/x.gd")));
    }
}
