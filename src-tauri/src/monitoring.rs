use crate::core::Service;
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};
use tauri::Emitter;

#[derive(Default)]
pub struct MonitorStatus {
    pub label: String,
    pub warnings: Vec<String>,
}

fn ordinary_directory(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}

// This bounded fingerprint is a fallback for dropped notify events and missing/replaced roots.
fn fingerprint(path: &Path, entries: &mut BTreeMap<PathBuf, (u64, u128)>, budget: &mut usize) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return;
        }
    }
    if meta.file_type().is_symlink() {
        return;
    }
    entries.insert(
        path.to_owned(),
        (
            meta.len(),
            meta.modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|m| m.as_nanos())
                .unwrap_or(0),
        ),
    );
    if meta.is_dir() {
        if let Ok(read) = fs::read_dir(path) {
            for entry in read.flatten() {
                fingerprint(&entry.path(), entries, budget);
            }
        }
    }
}

fn roots(snapshot: &Value) -> BTreeSet<PathBuf> {
    let mut result = BTreeSet::new();
    if let Some(games) = snapshot["games"].as_array() {
        for game in games {
            if let Some(folders) = game["folders"].as_array() {
                for folder in folders {
                    if let (Some(dir), Some(id)) = (folder["path"].as_str(), folder["id"].as_str())
                    {
                        let path = PathBuf::from(dir);
                        if let Some(parent) = path.parent() {
                            result.insert(parent.join(format!(".mod-manager-disabled-{id}")));
                        }
                        result.insert(path);
                    }
                }
            }
        }
    }
    result
}

fn missing_directories(snapshot: &Value) -> Vec<String> {
    snapshot["games"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|game| game["folders"].as_array().into_iter().flatten())
        .filter_map(|folder| folder["path"].as_str())
        .filter(|path| !ordinary_directory(Path::new(path)))
        .map(|path| format!("MOD 目录不可访问：{path}"))
        .collect()
}

#[derive(Default)]
struct MonitorFailures {
    watcher: Vec<String>,
    scan: Vec<String>,
    maintenance: Vec<String>,
    recovery_registered: bool,
}

impl MonitorFailures {
    fn watcher_failed(&mut self, error: impl std::fmt::Display) {
        self.watcher = vec![format!("目录监控暂时不可用，将定时核对文件：{error}")];
        self.recovery_registered = false;
    }

    fn observe_event(&mut self, covered: bool) {
        // A scan or registration success alone does not prove event delivery has recovered.
        if self.recovery_registered && covered {
            self.watcher.clear();
            self.recovery_registered = false;
        }
    }

    fn warnings(&self) -> Vec<String> {
        self.watcher
            .iter()
            .chain(&self.scan)
            .chain(&self.maintenance)
            .cloned()
            .collect()
    }
}

pub fn start(
    app: tauri::AppHandle,
    service: Arc<Mutex<Service>>,
    status: Arc<Mutex<MonitorStatus>>,
) {
    thread::spawn(move || {
        let (tx, rx) = mpsc::channel();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let _ = tx.send(event);
            })
            .ok();
        let mut watched = BTreeSet::<PathBuf>::new();
        let mut watched_identity = BTreeMap::new();
        let mut desired = BTreeSet::<PathBuf>::new();
        let mut previous = BTreeMap::new();
        let mut dirty_since: Option<Instant> = None;
        let mut last_scan = Instant::now();
        let mut last_check = Instant::now() - Duration::from_secs(10);
        let mut last_maintenance = Instant::now();
        let mut failures = MonitorFailures::default();
        let mut fallback = true;
        loop {
            thread::sleep(Duration::from_millis(500));
            for event in rx.try_iter() {
                match event {
                    Ok(event) if !matches!(event.kind, notify::EventKind::Access(_)) => {
                        dirty_since = Some(Instant::now());
                        let covered = !watched.is_empty()
                            && desired
                                .iter()
                                .filter(|path| ordinary_directory(path))
                                .all(|path| watched.contains(path))
                            && event
                                .paths
                                .iter()
                                .any(|path| watched.iter().any(|root| path.starts_with(root)));
                        failures.observe_event(covered);
                    }
                    Err(error) => {
                        failures.watcher_failed(error);
                    }
                    _ => {}
                }
            }
            if last_check.elapsed() >= Duration::from_secs(3) {
                let snapshot = service.try_lock().ok().map(|s| s.monitor_configuration());
                if let Some(snapshot) = snapshot {
                    let next = roots(&snapshot);
                    if next != desired {
                        dirty_since = Some(Instant::now());
                        desired = next;
                    }
                    let mut registration_errors = Vec::new();
                    if let Some(watcher) = watcher.as_mut() {
                        // Re-register every live root once after an event error. Keep the warning
                        // until full coverage and a subsequent real event demonstrate recovery.
                        if !failures.watcher.is_empty() && !failures.recovery_registered {
                            for path in &watched {
                                let _ = watcher.unwatch(path);
                            }
                            watched.clear();
                            watched_identity.clear();
                        }
                        let stale: Vec<_> = watched.difference(&desired).cloned().collect();
                        for path in stale {
                            let _ = watcher.unwatch(&path);
                            watched.remove(&path);
                            watched_identity.remove(&path);
                        }
                        for path in &desired {
                            if watched.contains(path)
                                && (!ordinary_directory(path)
                                    || watched_identity.get(path)
                                        != crate::core::disk_identity(path).as_ref())
                            {
                                let _ = watcher.unwatch(path);
                                watched.remove(path);
                                watched_identity.remove(path);
                            }
                            if !watched.contains(path) && ordinary_directory(path) {
                                match watcher.watch(path, RecursiveMode::Recursive) {
                                    Ok(()) => {
                                        watched.insert(path.clone());
                                        if let Some(identity) = crate::core::disk_identity(path) {
                                            watched_identity.insert(path.clone(), identity);
                                        }
                                    }
                                    Err(error) => registration_errors.push(format!(
                                        "实时监控注册失败，将定时核对 {}：{error}",
                                        path.display()
                                    )),
                                }
                            } else if path.is_dir() && !ordinary_directory(path) {
                                registration_errors.push(format!(
                                    "目录已变为链接或重解析点，停止监控：{}",
                                    path.display()
                                ));
                                if watched.remove(path) {
                                    let _ = watcher.unwatch(path);
                                }
                            }
                        }
                        if !failures.watcher.is_empty()
                            && registration_errors.is_empty()
                            && !watched.is_empty()
                            && desired
                                .iter()
                                .filter(|path| ordinary_directory(path))
                                .all(|path| watched.contains(path))
                        {
                            failures.recovery_registered = true;
                        }
                    }
                    let missing = missing_directories(&snapshot);
                    fallback = watcher.is_none()
                        || !registration_errors.is_empty()
                        || !failures.watcher.is_empty()
                        || !missing.is_empty();
                    let mut budget = 100_000;
                    // Healthy native events are the primary signal. Only the
                    // degraded path needs a recursive fingerprint every 3 s.
                    if fallback {
                        let mut current = BTreeMap::new();
                        for path in &desired {
                            fingerprint(path, &mut current, &mut budget);
                        }
                        if current != previous {
                            dirty_since = Some(Instant::now());
                            previous = current;
                        }
                    } else {
                        previous.clear();
                    }
                    let status_changed = if let Ok(mut status) = status.lock() {
                        let before = (status.label.clone(), status.warnings.clone());
                        status.label = if watcher.is_some()
                            && registration_errors.is_empty()
                            && failures.warnings().is_empty()
                            && missing.is_empty()
                        {
                            "实时监控 · 自动核对"
                        } else {
                            "定时监控 · 每 3 秒核对"
                        }
                        .into();
                        status.warnings = failures.warnings();
                        status.warnings.extend(registration_errors);
                        status.warnings.extend(missing);
                        if budget == 0 {
                            status.warnings.push(
                                "目录文件数量过多，部分变化依赖实时事件；可手动刷新核对".into(),
                            );
                        }
                        before != (status.label.clone(), status.warnings.clone())
                    } else {
                        false
                    };
                    // Emit after releasing the status lock; unchanged snapshots do not reload
                    // the interface, while directory recovery clears warnings promptly.
                    if status_changed {
                        let _ = app.emit("library-changed", ());
                    }
                }
                last_check = Instant::now();
            }
            // A quiet interval lets copies settle, and a periodic full scan repairs lost events.
            let quiet =
                dirty_since.is_some_and(|since| since.elapsed() >= Duration::from_millis(1800));
            if quiet
                || (!desired.is_empty()
                    && dirty_since.is_none()
                    && last_scan.elapsed() >= Duration::from_secs(if fallback { 15 } else { 60 }))
            {
                if let Ok(mut service) = service.try_lock() {
                    if let Err(error) = service.request(json!({"action":"scan"})) {
                        failures.scan = vec![format!("自动扫描未完成：{error}")];
                    } else {
                        failures.scan.clear();
                        let _ = app.emit("library-changed", ());
                    }
                    dirty_since = None;
                    last_scan = Instant::now();
                }
            }
            if last_maintenance.elapsed() >= Duration::from_secs(60 * 60) {
                if let Ok(mut service) = service.try_lock() {
                    match service.request(json!({"action":"maintenance"})) {
                        Ok(_) => failures.maintenance.clear(),
                        Err(error) => {
                            failures.maintenance = vec![format!("数据维护暂未完成：{error}")]
                        }
                    }
                    last_maintenance = Instant::now();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_recovery_preserves_unproven_watcher_error() {
        let mut failures = MonitorFailures::default();
        failures.watcher_failed("transient event failure");
        failures.scan.push("transient scan failure".into());
        failures.scan.clear();
        assert_eq!(failures.warnings().len(), 1);
        failures.observe_event(true);
        assert!(!failures.watcher.is_empty());
        failures.recovery_registered = true;
        failures.observe_event(false);
        assert!(!failures.watcher.is_empty());
        failures.observe_event(true);
        assert!(failures.warnings().is_empty());
    }

    #[test]
    fn repeated_watcher_failure_requires_new_recovery_evidence() {
        let mut failures = MonitorFailures::default();
        failures.watcher_failed("first");
        failures.recovery_registered = true;
        failures.watcher_failed("second");
        failures.observe_event(true);
        assert_eq!(failures.watcher.len(), 1);
        assert!(failures.watcher[0].contains("second"));
    }

    #[test]
    fn unavailable_secondary_folder_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let existing = temp.path().join("existing");
        fs::create_dir(&existing).unwrap();
        let missing = temp.path().join("secondary-missing");
        let snapshot =
            json!({"games":[{"modDir":existing,"folders":[{"path":existing},{"path":missing}]}]});
        let warnings = missing_directories(&snapshot);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("secondary-missing"));
    }
}
