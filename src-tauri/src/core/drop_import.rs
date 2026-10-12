use super::*;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const CANCELLED: &str = "导入已取消，源文件保持不变";
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

fn copy_bytes(
    input: &mut dyn Read,
    output: &mut dyn Write,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<u64, String> {
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut copied = 0;
    loop {
        check_cancel(cancel)?;
        let n = match input.read(&mut buffer) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            other => other.map_err(error)?,
        };
        if n == 0 {
            return Ok(copied);
        }
        let mut written = 0;
        while written < n {
            check_cancel(cancel)?;
            let bytes = match output.write(&buffer[written..n]) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                other => other.map_err(error)?,
            };
            if bytes == 0 {
                return Err("写入暂存文件失败：磁盘未接受数据".into());
            }
            written += bytes;
            copied += bytes as u64;
            progress(bytes as u64);
        }
    }
}

struct ImportProgress<'a> {
    callback: &'a mut dyn FnMut(Value),
    operation_id: String,
    copied_bytes: u64,
    total_bytes: u64,
    count: usize,
    index: usize,
    current_name: String,
    last_emit: Instant,
}
impl ImportProgress<'_> {
    fn emit(&mut self, phase: &str) {
        (self.callback)(
            json!({"operationId":self.operation_id,"phase":phase,"copiedBytes":self.copied_bytes,"totalBytes":self.total_bytes,"currentName":self.current_name,"index":self.index,"count":self.count}),
        );
        self.last_emit = Instant::now();
    }
    fn copied(&mut self, bytes: u64) {
        self.copied_bytes = self.copied_bytes.saturating_add(bytes);
        if self.copied_bytes == bytes || self.last_emit.elapsed() >= Duration::from_millis(100) {
            self.emit("copying");
        }
    }
}

#[derive(PartialEq, Eq)]
struct Entry {
    relative: PathBuf,
    directory: bool,
    identity: String,
    size: u64,
    modified: std::time::SystemTime,
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Service, String, String, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("mods");
        fs::create_dir(&root).unwrap();
        let mut service = Service::new(temp.path().join("data")).unwrap();
        let snapshot = service.request(json!({"action":"saveGame","game":{"name":"Import test","modDir":root,"strategy":"suffix"}})).unwrap();
        let game = snapshot["games"][0]["id"].as_str().unwrap().to_owned();
        let folder = snapshot["games"][0]["folders"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        (temp, service, game, folder, root)
    }
    #[test]
    fn cancellation_after_first_publish_keeps_it_and_all_sources() {
        let (temp, mut service, game, folder, root) = fixture();
        let a = temp.path().join("a.pak");
        let b = temp.path().join("b.pak");
        fs::write(&a, b"first").unwrap();
        fs::write(&b, b"second").unwrap();
        let cancel = AtomicBool::new(false);
        let mut events = vec![];
        let out = service
            .import_files_controlled(
                &json!({"gameId":game,"folderId":folder,"paths":[a,b]}),
                &cancel,
                &mut |event| {
                    if event["phase"] == "published" {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    events.push(event);
                },
            )
            .unwrap();
        assert_eq!(out["results"][0]["success"], true);
        assert_eq!(out["results"][1]["cancelled"], true);
        assert_eq!(out["copiedBytes"], 5);
        assert_eq!(out["totalBytes"], 11);
        assert_eq!(fs::read(root.join("a.pak")).unwrap(), b"first");
        assert!(!root.join("b.pak").exists());
        assert_eq!(fs::read(&a).unwrap(), b"first");
        assert_eq!(fs::read(&b).unwrap(), b"second");
        assert_eq!(events.last().unwrap()["phase"], "cancelled");
    }
    #[test]
    fn cancellation_during_copy_cleans_only_current_stage_with_persisted_inventory() {
        let (temp, mut service, game, folder, root) = fixture();
        let source = temp.path().join("large.pak");
        fs::write(&source, vec![42u8; 3 * 1024 * 1024]).unwrap();
        let cancel = AtomicBool::new(false);
        let out = service
            .import_files_controlled(
                &json!({"gameId":game,"folderId":folder,"paths":[source]}),
                &cancel,
                &mut |event| {
                    if event["copiedBytes"].as_u64().unwrap_or(0) > 0 {
                        cancel.store(true, Ordering::Relaxed);
                    }
                },
            )
            .unwrap();
        assert_eq!(out["results"][0]["cancelled"], true);
        assert_eq!(out["copiedBytes"], 1024 * 1024);
        assert!(!root.join("large.pak").exists());
        assert_eq!(fs::metadata(source).unwrap().len(), 3 * 1024 * 1024);
        let (stage, identity, inventory): (String, String, String) = service
            .conn
            .query_row(
                "SELECT stage,stage_identity,inventory FROM file_imports",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert!(!identity.is_empty());
        let inventory: Value = serde_json::from_str(&inventory).unwrap();
        assert_eq!(inventory.as_array().unwrap().len(), 1);
        assert!(!inventory[0]["identity"].as_str().unwrap().is_empty());
        assert!(!Path::new(&stage).exists());
    }
    #[test]
    fn stage_record_failure_prevents_copy_and_publication() {
        let (temp, mut service, game, folder, root) = fixture();
        let source = temp.path().join("keep.pak");
        fs::write(&source, b"keep").unwrap();
        service.conn.execute_batch("CREATE TRIGGER fail_stage BEFORE UPDATE OF stage_identity ON file_imports BEGIN SELECT RAISE(FAIL,'simulated stage journal failure'); END;").unwrap();
        let out = service
            .import_files_controlled(
                &json!({"gameId":game,"folderId":folder,"paths":[source]}),
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(out["copiedBytes"], 0);
        assert_eq!(out["results"][0]["success"], false);
        assert!(out["batchError"].as_str().unwrap().contains("操作记录"));
        assert!(!root.join("keep.pak").exists());
        assert_eq!(fs::read(source).unwrap(), b"keep");
    }
    #[test]
    fn byte_progress_counts_actual_partial_writes_and_disk_full_failure() {
        struct Limited {
            bytes: Vec<u8>,
            capacity: usize,
        }
        impl Write for Limited {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                let n = bytes.len().min(3).min(self.capacity - self.bytes.len());
                if n == 0 {
                    return Err(std::io::Error::other("simulated disk full"));
                }
                self.bytes.extend_from_slice(&bytes[..n]);
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut output = Limited {
            bytes: vec![],
            capacity: 7,
        };
        let mut progress = vec![];
        let result = copy_bytes(
            &mut &b"0123456789"[..],
            &mut output,
            &AtomicBool::new(false),
            &mut |n| progress.push(n),
        );
        assert!(result.unwrap_err().contains("simulated disk full"));
        assert_eq!(output.bytes, b"0123456");
        assert_eq!(progress, vec![3, 3, 1]);
    }
    #[test]
    fn cancellation_stops_mid_copy_before_reading_remaining_bytes() {
        let cancel = AtomicBool::new(false);
        let data = vec![42u8; 3 * 1024 * 1024];
        let mut output = vec![];
        let result = copy_bytes(&mut &data[..], &mut output, &cancel, &mut |_| {
            cancel.store(true, Ordering::Relaxed)
        });
        assert!(result.unwrap_err().contains("取消"));
        assert_eq!(output.len(), 1024 * 1024);
        assert_eq!(data.len(), 3 * 1024 * 1024);
    }
    #[test]
    fn changed_source_is_rejected_before_publish() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.pak");
        let copy = temp.path().join("copy.pak");
        fs::write(&source, "before").unwrap();
        let mut entries = vec![];
        inventory(&source, &source, &mut entries).unwrap();
        fs::write(&source, "different length").unwrap();
        let mut created = vec![];
        assert!(copy_inventory(&source, &copy, &entries, &mut created).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"different length");
    }
}
#[cfg(test)]
fn inventory(root: &Path, p: &Path, entries: &mut Vec<Entry>) -> Result<(), String> {
    inventory_controlled(root, p, entries, &AtomicBool::new(false))
}
fn inventory_controlled(
    root: &Path,
    p: &Path,
    entries: &mut Vec<Entry>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    check_cancel(cancel)?;
    reject_links(p)?;
    let meta = fs::symlink_metadata(p).map_err(error)?;
    if !meta.is_file() && !meta.is_dir() {
        return Err("只能导入普通文件和文件夹".into());
    }
    entries.push(Entry {
        relative: p.strip_prefix(root).map_err(error)?.to_path_buf(),
        directory: meta.is_dir(),
        identity: disk_identity(p).ok_or("无法核对源文件身份")?,
        size: if meta.is_file() { meta.len() } else { 0 },
        modified: meta.modified().map_err(error)?,
    });
    if meta.is_dir() {
        let mut children = fs::read_dir(p)
            .map_err(error)?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        children.sort();
        for child in children {
            inventory_controlled(root, &child, entries, cancel)?;
        }
    }
    Ok(())
}
#[cfg(test)]
fn copy_inventory(
    source: &Path,
    payload: &Path,
    entries: &[Entry],
    created: &mut Vec<(PathBuf, String, bool)>,
) -> Result<(), String> {
    copy_inventory_controlled(
        source,
        payload,
        entries,
        created,
        &AtomicBool::new(false),
        &mut |_| Ok(()),
        &mut |_| {},
    )
}
fn copy_inventory_controlled(
    source: &Path,
    payload: &Path,
    entries: &[Entry],
    created: &mut Vec<(PathBuf, String, bool)>,
    cancel: &AtomicBool,
    record: &mut dyn FnMut(&[(PathBuf, String, bool)]) -> Result<(), String>,
    progress: &mut dyn FnMut(u64),
) -> Result<(), String> {
    for entry in entries {
        check_cancel(cancel)?;
        let from = if entry.relative.as_os_str().is_empty() {
            source.to_path_buf()
        } else {
            source.join(&entry.relative)
        };
        let to = if entry.relative.as_os_str().is_empty() {
            payload.to_path_buf()
        } else {
            payload.join(&entry.relative)
        };
        reject_links(&from)?;
        reject_links(&to)?;
        if disk_identity(&from).as_deref() != Some(&entry.identity) {
            return Err("源文件在复制期间发生变化，请稍后重试".into());
        }
        if entry.directory {
            fs::create_dir(&to).map_err(error)?;
            created.push((
                to.clone(),
                disk_identity(&to).ok_or("无法核对暂存目录身份")?,
                true,
            ));
            record(created)?;
            continue;
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let mut input = options
            .open(&from)
            .map_err(|e| format!("无法读取源文件，可能仍在下载或被占用：{e}"))?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&to)
            .map_err(error)?;
        created.push((
            to.clone(),
            disk_identity(&to).ok_or("无法核对暂存文件身份")?,
            false,
        ));
        record(created)?;
        let copied = copy_bytes(&mut input, &mut output, cancel, progress)?;
        output.sync_all().map_err(error)?;
        if copied != entry.size
            || disk_identity(&from).as_deref() != Some(&entry.identity)
            || input.metadata().map_err(error)?.modified().map_err(error)? != entry.modified
        {
            return Err("源文件在复制期间发生变化，请稍后重试".into());
        }
    }
    let mut after = vec![];
    inventory_controlled(source, source, &mut after, cancel)?;
    if after != entries {
        return Err("源文件夹在复制期间发生变化，请稍后重试".into());
    }
    Ok(())
}
impl Service {
    pub(super) fn check_interrupted_imports(&mut self) -> Result<(), String> {
        let mut q = self
            .conn
            .prepare("SELECT stage,target FROM file_imports WHERE status NOT IN ('done','failed','resolved','acknowledged')")
            .map_err(error)?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(error)?;
        for row in rows {
            let (stage, target) = row.map_err(error)?;
            self.warnings.push(format!("上次拖放导入中断，请核对目标 {target}；暂存副本位于 {stage}。源文件未删除，可重试导入。"));
        }
        Ok(())
    }
    pub(super) fn import_files(&mut self, v: &Value) -> Result<Value, String> {
        self.import_files_controlled(v, &AtomicBool::new(false), &mut |_| {})
    }
    pub fn import_files_controlled(
        &mut self,
        v: &Value,
        cancel: &AtomicBool,
        callback: &mut dyn FnMut(Value),
    ) -> Result<Value, String> {
        self.ensure_no_journals()?;
        let id = string(v, "gameId")?;
        let folder_id = string(v, "folderId")?;
        let owner = self
            .lib
            .games
            .iter()
            .find(|g| g.id == id)
            .ok_or("游戏配置不存在")?;
        if Self::process_running(owner) {
            return Err(format!("{} 正在运行，请先退出游戏再导入 MOD", owner.name));
        }
        let scoped = folder_game(owner, &folder_id)?;
        let root = safe_root(&scoped.mod_dir)?;
        if let Some(expected) = v.get("expectedPath").and_then(Value::as_str) {
            if !same(&root, &PathBuf::from(expected)) {
                return Err("目标目录配置已变化，请重新拖入文件".into());
            }
        }
        let root_identity = disk_identity(&root).ok_or("无法核对目标目录身份")?;
        let paths = v
            .get("paths")
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty() && a.len() <= 256)
            .ok_or("每次请拖入 1 至 256 个文件或文件夹")?;
        let operation_id = v
            .get("operationId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(uid);
        let mut progress = ImportProgress {
            callback,
            operation_id: operation_id.clone(),
            copied_bytes: 0,
            total_bytes: 0,
            count: paths.len(),
            index: 0,
            current_name: String::new(),
            last_emit: Instant::now(),
        };
        progress.emit("preparing");
        let mut plans = vec![];
        let mut names = HashSet::new();
        // Validate the entire batch before making any copy. Never overwrite or merge folders.
        for p in paths {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let source = PathBuf::from(p.as_str().ok_or("拖入路径格式错误")?);
            if !source.is_absolute() {
                return Err("拖入文件应使用绝对路径".into());
            }
            // The cancellable inventory below checks every descendant before any copy;
            // check the selected path before canonicalization so a root link cannot hide.
            reject_links(&source)?;
            let source =
                fs::canonicalize(source).map_err(|e| format!("源文件不存在或无法读取：{e}"))?;
            if within(&source, &root) || within(&root, &source) {
                return Err("文件已经在当前 MOD 目录中，或包含该目录，无法重复导入".into());
            }
            let name = source.file_name().ok_or("不能导入磁盘根目录")?;
            let name_text = name.to_string_lossy();
            if name_text.starts_with(".mod-manager-") {
                return Err("不能导入管理器的暂存目录".into());
            }
            if !names.insert(name_text.to_lowercase()) {
                return Err("拖入的项目包含重复名称，请分开处理".into());
            }
            let target = root.join(name);
            if fs::symlink_metadata(&target).is_ok() {
                return Err(format!("目标已存在，未覆盖：{}", target.display()));
            }
            let mut entries = vec![];
            if let Err(e) = inventory_controlled(&source, &source, &mut entries, cancel) {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                return Err(e);
            }
            progress.total_bytes = entries
                .iter()
                .try_fold(progress.total_bytes, |sum, entry| {
                    sum.checked_add(entry.size)
                })
                .ok_or("导入数据量超出支持范围")?;
            plans.push((source, target, entries));
        }
        if cancel.load(Ordering::Relaxed) {
            progress.emit("cancelled");
            let results: Vec<_> = paths.iter().map(|p| json!({"name":p.as_str().and_then(|p| Path::new(p).file_name()).unwrap_or_default().to_string_lossy(),"success":false,"cancelled":true,"error":CANCELLED})).collect();
            return Ok(
                json!({"operationId":operation_id,"cancelled":true,"copiedBytes":0,"totalBytes":progress.total_bytes,"results":results,"scanError":null,"batchError":null,"snapshot":self.snapshot()}),
            );
        }
        let mut results = vec![];
        let mut batch_error: Option<String> = None;
        for (index, (source, target, entries)) in plans.into_iter().enumerate() {
            progress.index = index + 1;
            progress.current_name = source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if cancel.load(Ordering::Relaxed) {
                results.push(json!({"name":progress.current_name,"success":false,"cancelled":true,"error":CANCELLED}));
                continue;
            }
            if batch_error.is_some() {
                results.push(json!({"name":source.file_name().unwrap_or_default().to_string_lossy(),"success":false,"error":"未开始：操作记录无法保存，已停止剩余导入"}));
                continue;
            }
            let operation = uid();
            let stage = root
                .parent()
                .ok_or("无法创建暂存目录")?
                .join(format!(".mod-manager-import-{operation}"));
            if self
                .lib
                .games
                .iter()
                .flat_map(|g| &g.folders)
                .any(|f| within(&stage, &f.path))
            {
                return Err("无法在加载范围外创建暂存目录".into());
            }
            if let Err(e)=self.conn.execute("INSERT INTO file_imports(id,source,target,stage,status) VALUES(?1,?2,?3,?4,'pending')",params![operation,source.to_string_lossy(),target.to_string_lossy(),stage.to_string_lossy()]) {
                batch_error=Some(format!("操作记录无法保存，已停止剩余导入。请检查磁盘空间和数据目录权限：{e}"));
                results.push(json!({"name":source.file_name().unwrap_or_default().to_string_lossy(),"success":false,"error":"未开始：无法保存操作记录"}));
                continue;
            }
            let mut owned = None;
            let mut created = vec![];
            let mut persistence_error: Option<String> = None;
            progress.emit("copying");
            let result: Result<(), String> = (|| {
                check_cancel(cancel)?;
                reject_links(&stage)?;
                fs::create_dir(&stage).map_err(error)?;
                owned = disk_identity(&stage);
                if owned.is_none() {
                    return Err("无法核对暂存目录身份".into());
                }
                if let Err(e) = self.conn.execute(
                    "UPDATE file_imports SET stage_identity=?2,inventory='[]' WHERE id=?1",
                    params![operation, owned],
                ) {
                    let message = format!("操作记录无法保存，未开始复制：{e}");
                    persistence_error = Some(message.clone());
                    return Err(message);
                }
                let payload = stage.join("payload");
                copy_inventory_controlled(
                    &source,
                    &payload,
                    &entries,
                    &mut created,
                    cancel,
                    &mut |members| {
                        let inventory: Vec<_> = members.iter().map(|(path,identity,directory)| json!({"path":path,"identity":identity,"directory":directory})).collect();
                        self.conn
                            .execute(
                                "UPDATE file_imports SET inventory=?2 WHERE id=?1",
                                params![
                                    operation,
                                    serde_json::to_string(&inventory).map_err(error)?
                                ],
                            )
                            .map_err(|e| {
                                let message = format!("操作记录无法保存，已停止复制：{e}");
                                persistence_error = Some(message.clone());
                                message
                            })?;
                        Ok(())
                    },
                    &mut |bytes| progress.copied(bytes),
                )?;
                progress.emit("publishing");
                check_cancel(cancel)?;
                if disk_identity(&root).as_deref() != Some(&root_identity) {
                    return Err("目标目录发生变化，未导入，请重新选择目录".into());
                }
                reject_links(&root)?;
                no_replace_move(&payload, &target)?;
                progress.emit("published");
                Ok(())
            })();
            // Only clean our newly created temporary copies, never source or target contents.
            let cleanup = (|| {
                if owned.is_none() {
                    return if fs::symlink_metadata(&stage).is_ok() {
                        Err("暂存目录身份未记录，已保留".to_string())
                    } else {
                        Ok(())
                    };
                }
                if disk_identity(&stage) != owned {
                    return Err("暂存目录身份变化，已保留".to_string());
                }
                reject_links(&stage)?;
                for (path, identity, directory) in created.iter().rev() {
                    if fs::symlink_metadata(path).is_err() {
                        continue;
                    }
                    reject_links(path)?;
                    if disk_identity(path).as_deref() != Some(identity) {
                        return Err("暂存成员变化，已保留".to_string());
                    }
                    if *directory {
                        fs::remove_dir(path).map_err(error)?;
                    } else {
                        fs::remove_file(path).map_err(error)?;
                    }
                }
                fs::remove_dir(&stage).map_err(error)
            })();
            let mut detail = result.as_ref().err().cloned();
            let cleanup_failed = cleanup.is_err();
            if let Err(e) = cleanup {
                detail = Some(match detail {
                    Some(detail) => format!("{detail}；暂存清理未完成：{e}"),
                    None => format!("导入完成，但暂存清理未完成：{e}"),
                });
                self.warnings
                    .push(format!("暂存副本未清理：{}（{e}）", stage.display()));
            }
            if let Err(e) = self.conn.execute(
                "UPDATE file_imports SET status=?2,detail=?3 WHERE id=?1",
                params![
                    operation,
                    if cleanup_failed {
                        "cleanup_failed"
                    } else if result.is_ok() {
                        "done"
                    } else {
                        "failed"
                    },
                    detail
                ],
            ) {
                batch_error=Some(format!("操作记录无法保存，已停止剩余导入。已复制成功的文件仍保留在目标目录，请核对列表：{e}"));
            }
            if let Some(e) = persistence_error {
                batch_error = Some(e);
            }
            results.push(json!({"name":source.file_name().unwrap_or_default().to_string_lossy(),"success":result.is_ok(),"cancelled":result.as_ref().err().is_some_and(|e| e == CANCELLED),"error":detail}));
        }
        let scan_error = self.scan(Some(&id)).err();
        let cancelled = cancel.load(Ordering::Relaxed);
        progress.emit(if cancelled { "cancelled" } else { "completed" });
        Ok(
            json!({"operationId":operation_id,"cancelled":cancelled,"copiedBytes":progress.copied_bytes,"totalBytes":progress.total_bytes,"results":results,"scanError":scan_error,"batchError":batch_error,"snapshot":self.snapshot()}),
        )
    }
}
