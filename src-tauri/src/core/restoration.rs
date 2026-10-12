use super::*;

impl Service {
    pub(super) fn recent_deletions(&self) -> Vec<Value> {
        let Ok(mut q)=self.conn.prepare("SELECT id,data,status,at FROM deletions WHERE status IN ('done','partial','interrupted','restoring') ORDER BY rowid DESC LIMIT 8") else { return vec![]; };
        let Ok(rows) = q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        }) else {
            return vec![];
        };
        rows.filter_map(Result::ok).filter_map(|(id,raw,status,at)| {
            let m:Mod=serde_json::from_str(&raw).ok()?;
            self.lib.games.iter().find(|g|g.id==m.game_id)?;
            Some(json!({"id":id,"name":m.members.first().and_then(|f|f.current_path.file_name()).map(|s|s.to_string_lossy()),"status":status,"at":at,"gameId":m.game_id,"folderId":m.folder_id}))
        }).collect()
    }
    pub(super) fn restore_mod(&mut self, v: &Value) -> Result<(), String> {
        self.ensure_no_journals()?;
        let operation = string(v, "operationId")?;
        let (raw, status): (String, String) = self
            .conn
            .query_row(
                "SELECT data,status FROM deletions WHERE id=?1",
                params![operation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "没有可恢复的删除记录")?;
        if !["done", "partial", "interrupted", "restoring"].contains(&status.as_str()) {
            return Err("此记录已经恢复或未完成删除".into());
        }
        let m: Mod = serde_json::from_str(&raw).map_err(error)?;
        let audit: Value = serde_json::from_str(&raw).map_err(error)?;
        let g = self
            .lib
            .games
            .iter()
            .find(|g| g.id == m.game_id)
            .ok_or("原游戏配置已移除，不能自动恢复")?;
        let g = folder_game(g, &m.folder_id)?;
        reject_links(&g.mod_dir)?;
        if Self::process_running(&g) {
            return Err("游戏正在运行，请退出游戏后再恢复 MOD".into());
        }
        // Revalidate the original record against today's configuration, never trust journal paths.
        let mut next = self.lib.clone();
        next.mods.retain(|x| x.id != m.id);
        next.mods.push(m.clone());
        validate_library(&next)?;
        for f in &m.members {
            if self
                .lib
                .mods
                .iter()
                .filter(|x| x.id != m.id && !x.retired)
                .flat_map(|x| &x.members)
                .any(|x| {
                    within(&x.current_path, &f.current_path)
                        || within(&f.current_path, &x.current_path)
                })
            {
                return Err("恢复路径已被其他 MOD 使用，未覆盖文件".into());
            }
            let parent = f.current_path.parent().ok_or("恢复位置无效")?;
            if !parent.is_dir() {
                return Err(format!("恢复目录不存在：{}", parent.display()));
            }
            reject_links(parent)?;
            if f.current_path.exists() && disk_identity(&f.current_path) != f.disk_identity {
                return Err(format!(
                    "同名文件已经存在，未覆盖：{}",
                    f.current_path.display()
                ));
            }
            let counterpart = if same(&f.current_path, &f.original_path) {
                suffix_disabled(&f.original_path)
            } else {
                f.original_path.clone()
            };
            if f.kind == "file" && counterpart.exists() {
                return Err(format!("原始与禁用文件存在冲突：{}", counterpart.display()));
            }
        }
        let absent: Vec<Member> = m
            .members
            .iter()
            .filter(|f| f.exists && !f.current_path.exists())
            .cloned()
            .collect();
        let items = find_recycled(&absent)?;
        if items.len() != absent.len() {
            return Err(
                "部分文件已不在回收站，可能已清空或被移动；恢复记录已保留，可打开回收站检查".into(),
            );
        }
        for (member, item) in absent.iter().zip(&items) {
            reject_tree_links(&item.path)?;
            if disk_identity(&item.path) != member.disk_identity {
                return Err("回收站文件身份改变，未恢复".into());
            }
            for leaf in &member.leaves {
                let p = item.path.join(
                    leaf.current
                        .strip_prefix(&member.current_path)
                        .map_err(error)?,
                );
                let absent_before = audit["members"]
                    .as_array()
                    .and_then(|members| {
                        members
                            .iter()
                            .find(|f| f["id"].as_str() == Some(&member.id))
                    })
                    .and_then(|f| f["leaves"].as_array())
                    .and_then(|leaves| {
                        leaves.iter().find(|l| {
                            l["original"]
                                .as_str()
                                .is_some_and(|p| same(Path::new(p), &leaf.original))
                        })
                    })
                    .is_some_and(|l| l["existsAtDeletion"].as_bool() == Some(false));
                if absent_before && !p.exists() {
                    continue;
                }
                if disk_identity(&p) != leaf.identity {
                    return Err("回收站中的内部文件发生变化，请先核对".into());
                }
            }
        }
        let planned = serde_json::to_string(&next).map_err(error)?;
        let tx = self.conn.transaction().map_err(error)?;
        tx.execute("UPDATE library SET data=?1 WHERE id=1", params![planned])
            .map_err(error)?;
        tx.execute(
            "UPDATE deletions SET status='restoring' WHERE id=?1",
            params![operation],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        self.lib = next.clone();
        // Each restoration is an atomic same-volume move without overwrite. On failure keep
        // already restored members and the durable record; retry recognizes their identities.
        for (member, item) in absent.iter().zip(&items) {
            if disk_identity(&item.path) != member.disk_identity {
                return Err("恢复前文件身份改变，已停止".into());
            }
            no_replace_move(&item.path, &member.current_path).map_err(|e| {
                format!(
                    "恢复未完成，已恢复的文件保留；可重试。{}：{e}",
                    member.current_path.display()
                )
            })?;
            if disk_identity(&member.current_path) != member.disk_identity {
                return Err("恢复后文件身份核对失败，已保留记录".into());
            }
            // $I is Windows' metadata for this exact $R item, never Mod content. Remove only
            // an unchanged, verified metadata companion after the content has been restored.
            if let Some((meta, bytes)) = &item.metadata {
                if fs::read(meta).ok().as_ref() == Some(bytes) {
                    let _ = fs::remove_file(meta);
                }
            }
        }
        for f in m.members.iter().filter(|f| f.exists) {
            if disk_identity(&f.current_path) != f.disk_identity {
                return Err("部分成员尚未恢复，管理信息保留在恢复记录中".into());
            }
        }
        let raw = serde_json::to_string(&next).map_err(error)?;
        let tx = self.conn.transaction().map_err(error)?;
        tx.execute("UPDATE library SET data=?1 WHERE id=1", params![raw])
            .map_err(error)?;
        tx.execute(
            "UPDATE deletions SET status='restored' WHERE id=?1",
            params![operation],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        self.lib = next;
        self.reconcile();
        self.persist()
    }
}

pub(super) struct Recycled {
    pub(super) path: PathBuf,
    metadata: Option<(PathBuf, Vec<u8>)>,
}

#[cfg(windows)]
pub(super) fn find_recycled(members: &[Member]) -> Result<Vec<Recycled>, String> {
    if members.is_empty() {
        return Ok(vec![]);
    }
    let members = members.to_vec();
    std::thread::spawn(move || {
        use windows::Win32::{
            System::Com::{
                CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{
                BHID_EnumItems, FOLDERID_RecycleBinFolder, IEnumShellItems, IShellItem,
                SHGetKnownFolderItem, KF_FLAG_DEFAULT, SIGDN_FILESYSPATH,
            },
        };
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(error)?;
            struct Guard;
            impl Drop for Guard {
                fn drop(&mut self) {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
            let _guard = Guard;
            let bin: IShellItem =
                SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)
                    .map_err(error)?;
            let list: IEnumShellItems = bin.BindToHandler(None, &BHID_EnumItems).map_err(error)?;
            let mut paths = vec![];
            loop {
                let mut batch = [None];
                let mut count = 0;
                list.Next(&mut batch, Some(&mut count)).map_err(error)?;
                if count == 0 {
                    break;
                }
                if let Some(item) = batch[0].take() {
                    if let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                        let text = name.to_string();
                        CoTaskMemFree(Some(name.0.cast()));
                        if let Ok(text) = text {
                            paths.push(PathBuf::from(text));
                        }
                    }
                }
            }
            let mut result = vec![];
            for f in &members {
                let candidates: Vec<_> = paths
                    .iter()
                    .filter(|p| f.disk_identity.is_some() && disk_identity(p) == f.disk_identity)
                    .collect();
                if candidates.len() != 1 {
                    return Err("不能唯一定位回收站中的原文件，未恢复".into());
                }
                let p = candidates[0];
                let name = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or("回收站路径无效")?;
                let parent = p.parent().ok_or("回收站路径无效")?;
                if !name.starts_with("$R") || !norm(p).contains("\\$recycle.bin\\") {
                    return Err("回收站返回了不支持的存储位置，未恢复".into());
                }
                let metadata = parent.join(format!("$I{}", &name[2..]));
                reject_links(&metadata)?;
                let bytes = fs::read(&metadata).map_err(|_| "回收站元信息缺失，未恢复")?;
                if !metadata_matches(&bytes, &f.current_path) {
                    return Err("回收站原路径无法确认，未恢复".into());
                }
                result.push(Recycled {
                    path: p.clone(),
                    metadata: Some((metadata, bytes)),
                });
            }
            Ok(result)
        }
    })
    .join()
    .map_err(|_| "回收站读取失败".to_string())?
}

fn metadata_matches(bytes: &[u8], expected: &Path) -> bool {
    if bytes.len() < 24 {
        return false;
    }
    let version = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
    let start = match version {
        1 => 24,
        2 if bytes.len() >= 28 => 28,
        _ => return false,
    };
    let path: Vec<u16> = bytes[start..]
        .chunks_exact(2)
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
        .take_while(|v| *v != 0)
        .collect();
    String::from_utf16(&path)
        .ok()
        .is_some_and(|text| same(Path::new(&text), expected))
}
#[cfg(not(windows))]
fn find_recycled(_: &[Member]) -> Result<Vec<Recycled>, String> {
    Err("恢复回收站文件仅支持 Windows".into())
}
