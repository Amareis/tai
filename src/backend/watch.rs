use std::collections::HashMap;

use tracing::info;

use crate::backend::WindowId;

struct TrackedWindow {
    id: WindowId,
    title: String,
    command: String,
}

pub struct Watcher {
    tracked: HashMap<WindowId, TrackedWindow>,
}

impl Default for Watcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Watcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tracked: HashMap::new(),
        }
    }

    pub fn track(&mut self, id: WindowId, title: String, command: String) {
        info!("watcher: tracking {id} title={title}");
        self.tracked
            .insert(id.clone(), TrackedWindow { id, title, command });
    }

    #[must_use]
    pub fn watch_entries(&self) -> Vec<(WindowId, String, String)> {
        self.tracked
            .values()
            .map(|tw| (tw.id.clone(), tw.title.clone(), tw.command.clone()))
            .collect()
    }

    pub fn remove_by_title(&mut self, title: &str) {
        self.tracked.retain(|_, tw| tw.title != title);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::WindowId;

    #[test]
    fn test_watch_entries() {
        let mut watcher = Watcher::new();
        assert!(watcher.watch_entries().is_empty());

        watcher.track(
            WindowId::new("w1"),
            "task".to_string(),
            "cat TASK.md".to_string(),
        );
        watcher.track(
            WindowId::new("w2"),
            "tree".to_string(),
            "tree --gitignore".to_string(),
        );

        let entries = watcher.watch_entries();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_remove_by_title() {
        let mut watcher = Watcher::new();
        watcher.track(
            WindowId::new("w1"),
            "task".to_string(),
            "cat TASK.md".to_string(),
        );
        watcher.track(
            WindowId::new("w2"),
            "tree".to_string(),
            "tree --gitignore".to_string(),
        );

        watcher.remove_by_title("task");

        let entries = watcher.watch_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, "tree");
    }
}
