use super::*;

impl Service {
    pub(super) fn forget_mod(&mut self, v: &Value) -> Result<(), String> {
        self.ensure_no_journals()?;
        if v.get("confirmed").and_then(Value::as_bool) != Some(true) {
            return Err("请先确认移除 MOD 记录".into());
        }
        self.reconcile();
        let id = string(v, "id")?;
        let m = self
            .lib
            .mods
            .iter()
            .find(|m| m.id == id)
            .ok_or("MOD 不存在")?;
        if !m.retired && m.members.iter().any(|f| f.exists) {
            return Err("此 MOD 仍有真实文件，请使用移入回收站".into());
        }
        self.lib.mods.retain(|m| m.id != id);
        self.persist()
    }

    pub(super) fn delete_mod(&mut self, v: &Value) -> Result<(), String> {
        self.ensure_no_journals()?;
        if v.get("confirmed").and_then(Value::as_bool) != Some(true) {
            return Err("请先确认将 MOD 移入回收站".into());
        }
        self.reconcile();
        let id = string(v, "id")?;
        let m = self
            .lib
            .mods
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or("MOD 不存在")?;
        let expected = v
            .get("expectedMembers")
            .and_then(Value::as_array)
            .ok_or("请重新打开删除确认窗口，核对全部成员")?;
        if expected.len() != m.members.len()
            || m.members.iter().any(|f| {
                !expected.iter().any(|e| {
                    e["id"].as_str() == Some(&f.id)
                        && e["currentPath"]
                            .as_str()
                            .is_some_and(|p| same(Path::new(p), &f.current_path))
                        && e["exists"].as_bool() == Some(f.exists)
                })
            })
        {
            return Err("MOD 成员或状态已改变，请重新打开详情并确认删除范围".into());
        }
        if m.retired {
            return Err("历史记录只能移除管理信息，不会操作旧目录文件".into());
        }
        if m.state == "error" {
            return Err("MOD 文件存在冲突或身份改变，请先核对后再删除".into());
        }
        let g = self
            .lib
            .games
            .iter()
            .find(|g| g.id == m.game_id)
            .ok_or("游戏不存在")?;
        let g = folder_game(g, &m.folder_id)?;
        if Self::process_running(&g) {
            return Err("游戏正在运行，请退出游戏后再删除 MOD".into());
        }
        reject_links(&g.mod_dir)?;
        reject_links(&disabled_root(&g))?;
        let mut paths = Vec::new();
        for f in &m.members {
            if !within(&f.original_path, &g.mod_dir) || same(&f.original_path, &g.mod_dir) {
                return Err("MOD 成员超出授权目录".into());
            }
            let off = if g.strategy == "suffix" && f.kind == "file" {
                suffix_disabled(&f.original_path)
            } else {
                disabled_root(&g).join(f.original_path.strip_prefix(&g.mod_dir).map_err(error)?)
            };
            if !same(&f.current_path, &f.original_path) && !same(&f.current_path, &off) {
                return Err("当前文件路径不符合目录配置".into());
            }
            if !f.exists {
                continue;
            }
            reject_tree_links(&f.current_path)?;
            let metadata = fs::symlink_metadata(&f.current_path).map_err(error)?;
            if (f.kind == "file" && !metadata.is_file())
                || (f.kind == "directory" && !metadata.is_dir())
            {
                return Err("成员类型已经改变，请刷新核对".into());
            }
            if f.disk_identity.is_none() || disk_identity(&f.current_path) != f.disk_identity {
                return Err("成员文件身份无法确认，已阻止删除".into());
            }
            for other in self
                .lib
                .mods
                .iter()
                .filter(|other| !other.retired && other.id != id)
                .flat_map(|other| other.members.iter())
            {
                if within(&other.current_path, &f.current_path)
                    || within(&f.current_path, &other.current_path)
                {
                    return Err("此成员与另一个 MOD 的文件范围重叠，已阻止删除".into());
                }
            }
            paths.push(f.current_path.clone());
        }
        if paths.is_empty() {
            return Err("文件已全部缺失，请改为移除记录".into());
        }
        for (i, p) in paths.iter().enumerate() {
            if paths
                .iter()
                .enumerate()
                .any(|(j, q)| i != j && (within(p, q) || within(q, p)))
            {
                return Err("组内成员路径重叠，不能安全删除".into());
            }
        }
        let operation = uid();
        let mut audit = serde_json::to_value(&m).map_err(error)?;
        if let Some(members) = audit["members"].as_array_mut() {
            for member in members {
                if let Some(leaves) = member["leaves"].as_array_mut() {
                    for leaf in leaves {
                        leaf["existsAtDeletion"] = json!(leaf["current"]
                            .as_str()
                            .is_some_and(|p| Path::new(p).exists()));
                    }
                }
            }
        }
        self.conn
            .execute(
                "INSERT INTO deletions(id,data,status) VALUES(?1,?2,'pending')",
                params![operation, serde_json::to_string(&audit).map_err(error)?],
            )
            .map_err(error)?;
        // Recheck immediately before invoking the Shell. No raw delete or fallback is permitted.
        let result = if m
            .members
            .iter()
            .filter(|f| f.exists)
            .any(|f| disk_identity(&f.current_path) != f.disk_identity)
        {
            Err("文件在操作前被替换，未删除".into())
        } else {
            recycle_paths(paths.clone())
        };
        self.reconcile();
        let removed = paths
            .iter()
            .filter(|p| p.try_exists().ok() == Some(false))
            .count();
        let complete = self
            .lib
            .mods
            .iter()
            .find(|item| item.id == id)
            .is_some_and(|item| item.members.iter().all(|f| !f.exists))
            && removed == paths.len();
        if complete {
            self.lib.mods.retain(|m| m.id != id);
            self.persist()?;
            self.conn
                .execute(
                    "UPDATE deletions SET status='done' WHERE id=?1",
                    params![operation],
                )
                .map_err(error)?;
            if let Err(detail) = result {
                return Err(format!(
                    "成员已离开原目录，但 Windows 未确认回收完成。请检查回收站：{detail}"
                ));
            }
            return Ok(());
        }
        self.persist()?;
        self.conn
            .execute(
                "UPDATE deletions SET status=?2 WHERE id=?1",
                params![operation, if removed > 0 { "partial" } else { "failed" }],
            )
            .map_err(error)?;
        let detail = result
            .err()
            .unwrap_or_else(|| "部分成员仍在原位置，请检查占用或权限".into());
        Err(format!("{removed}/{} 个成员已离开原位置。剩余文件和管理记录保留；可在回收站恢复已移出的成员。{detail}",paths.len()))
    }

    pub(super) fn recover_deletions(&mut self) -> Result<(), String> {
        let restoring: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM deletions WHERE status='restoring'",
                [],
                |r| r.get(0),
            )
            .map_err(error)?;
        if restoring > 0 {
            self.warnings.push(
                "上次恢复未完成，已保留成员与管理信息；请在设置的最近删除中核对并继续恢复".into(),
            );
        }
        let records = {
            let mut q = self
                .conn
                .prepare("SELECT id,data FROM deletions WHERE status='pending'")
                .map_err(error)?;
            let rows = q
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(error)?
        };
        self.reconcile();
        for (id, raw) in records {
            let m: Mod = serde_json::from_str(&raw).map_err(error)?;
            let all_gone = self
                .lib
                .mods
                .iter()
                .find(|current| current.id == m.id)
                .is_some_and(|current| current.members.iter().all(|f| !f.exists))
                && m.members
                    .iter()
                    .all(|f| f.current_path.try_exists().ok() == Some(false));
            if all_gone {
                self.lib.mods.retain(|current| current.id != m.id);
                self.warnings.push(
                    "上次删除被中断，成员已离开原目录；请检查 Windows 回收站确认恢复位置".into(),
                );
            } else {
                self.warnings.push(
                    "上次 MOD 删除被中断，已核对磁盘并保留记录；请查看成员状态或在回收站恢复"
                        .into(),
                );
            }
            self.persist()?;
            self.conn
                .execute(
                    "UPDATE deletions SET status=?2 WHERE id=?1",
                    params![id, if all_gone { "done" } else { "interrupted" }],
                )
                .map_err(error)?;
        }
        Ok(())
    }
}

#[cfg(windows)]
fn recycle_paths(paths: Vec<PathBuf>) -> Result<(), String> {
    std::thread::spawn(move || {
        use windows::{
            core::{HSTRING, PCWSTR},
            Win32::{
                Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW},
                System::Com::{
                    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                    COINIT_APARTMENTTHREADED,
                },
                UI::Shell::{
                    FileOperation, IFileOperation, IFileOperationProgressSink, IShellItem,
                    SHCreateItemFromParsingName, FOFX_ADDUNDORECORD, FOFX_EARLYFAILURE,
                    FOFX_RECYCLEONDELETE, FOF_NOCONFIRMATION, FOF_NOERRORUI,
                    FOF_NO_CONNECTED_ELEMENTS, FOF_SILENT,
                },
            },
        };
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| format!("无法连接 Windows 回收站：{e}"))?;
            struct ComGuard;
            impl Drop for ComGuard {
                fn drop(&mut self) {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
            let _guard = ComGuard;
            let operation: IFileOperation =
                CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| format!("回收站不可用：{e}"))?;
            operation
                .SetOperationFlags(
                    FOFX_RECYCLEONDELETE
                        | FOFX_ADDUNDORECORD
                        | FOFX_EARLYFAILURE
                        | FOF_NOERRORUI
                        | FOF_SILENT
                        | FOF_NOCONFIRMATION
                        | FOF_NO_CONNECTED_ELEMENTS,
                )
                .map_err(error)?;
            for path in paths {
                // Shell parsing uses ordinary Windows paths rather than extended-length prefixes.
                let path = path.to_string_lossy();
                let ordinary = if let Some(tail) = path.strip_prefix("\\\\?\\UNC\\") {
                    format!("\\\\{tail}")
                } else {
                    path.strip_prefix("\\\\?\\").unwrap_or(&path).to_string()
                };
                let wide = HSTRING::from(ordinary);
                let mut volume = [0u16; 32768];
                GetVolumePathNameW(&wide, &mut volume)
                    .map_err(|_| "无法核对磁盘回收站，文件未删除")?;
                if GetDriveTypeW(PCWSTR(volume.as_ptr())) != 3 {
                    return Err(
                        "仅支持本地固定磁盘的回收站；网络盘或可移动磁盘上的文件不会被永久删除"
                            .into(),
                    );
                }
                let item: IShellItem = SHCreateItemFromParsingName(&wide, None)
                    .map_err(|e| format!("无法访问待删除成员：{e}"))?;
                operation
                    .DeleteItem(&item, None::<&IFileOperationProgressSink>)
                    .map_err(error)?;
            }
            operation
                .PerformOperations()
                .map_err(|e| format!("移入回收站失败，请检查文件占用和权限：{e}"))?;
            if operation
                .GetAnyOperationsAborted()
                .map_err(error)?
                .as_bool()
            {
                return Err("回收站操作被取消或部分成员未处理".into());
            }
            Ok(())
        }
    })
    .join()
    .map_err(|_| "回收站操作异常，已保留记录，请刷新核对文件状态".to_string())?
}
#[cfg(not(windows))]
fn recycle_paths(_: Vec<PathBuf>) -> Result<(), String> {
    Err("仅 Windows 支持移入回收站".into())
}
