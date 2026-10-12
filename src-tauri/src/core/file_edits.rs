use super::*;
#[derive(Serialize, Deserialize)]
struct EditRecord {
    before: Library,
    steps: Vec<Step>,
    identities: Vec<String>,
}

fn files(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    walk_files(root, out, 0)
}
fn walk_files(root: &Path, out: &mut Vec<PathBuf>, depth: usize) -> Result<(), String> {
    if depth > 128 || out.len() > 100_000 {
        return Err("MOD 文件夹层级过深或文件过多，请拆分后再管理".into());
    }
    reject_links(root)?;
    for entry in fs::read_dir(root).map_err(error)? {
        let p = entry.map_err(error)?.path();
        reject_links(&p)?;
        let meta = fs::symlink_metadata(&p).map_err(error)?;
        if meta.is_dir() {
            walk_files(&p, out, depth + 1)?;
        } else if meta.is_file() {
            out.push(p);
        } else {
            return Err("目录包含不支持的文件类型".into());
        }
    }
    Ok(())
}
pub(super) fn directory_state(f: &mut Member) -> Result<(usize, usize, usize), String> {
    let mut paths = vec![];
    files(&f.current_path, &mut paths)?;
    let mut known: HashSet<String> = f.leaves.iter().map(|l| norm(&l.original)).collect();
    for p in paths {
        let original = suffix_original(&p);
        if known.insert(norm(&original)) {
            f.leaves.push(Leaf {
                original,
                current: p.clone(),
                identity: disk_identity(&p),
                disabled_by_manager: Some(false),
            });
        }
    }
    let (mut on, mut off, mut missing, mut preserved) = (0, 0, 0, 0);
    for l in &mut f.leaves {
        let disabled = suffix_disabled(&l.original);
        match (
            l.original.try_exists().map_err(error)?,
            disabled.try_exists().map_err(error)?,
        ) {
            (true, true) => return Err("文件夹内存在原始与禁用文件同名冲突".into()),
            (true, false) => {
                l.current = l.original.clone();
                l.disabled_by_manager = Some(false);
                on += 1;
            }
            (false, true) => {
                l.current = disabled;
                if l.disabled_by_manager == Some(false) {
                    preserved += 1;
                } else {
                    off += 1;
                }
            }
            (false, false) => {
                missing += 1;
                continue;
            }
        }
        reject_links(&l.current)?;
        if l.identity.is_none() || disk_identity(&l.current) != l.identity {
            return Err("文件夹内有文件被替换，无法核对身份".into());
        }
    }
    if f.leaves.is_empty() {
        on = 1;
    } else if on == 0 && off == 0 && preserved > 0 {
        // A folder consisting only of externally disabled files really is disabled.
        off = preserved;
    }
    Ok((on, off, missing))
}
// NTFS may retain the creation time of a recently deleted destination (name tunneling).
// For a rename we just performed, volume and file ID establish continuity.
fn moved_identity(actual: Option<String>, expected: &str) -> bool {
    let Some(actual) = actual else { return false };
    #[cfg(windows)]
    {
        actual.split(':').take(3).eq(expected.split(':').take(3))
    }
    #[cfg(not(windows))]
    {
        actual == expected
    }
}
fn valid_name(name: &str) -> Result<(), String> {
    let stem = name.split('.').next().unwrap_or("").to_uppercase();
    if name.trim().is_empty()
        || name.encode_utf16().count() > 240
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
        || name.to_lowercase().ends_with(".disabled")
    {
        return Err("名称不能包含路径、Windows 保留字符或 .disabled 后缀".into());
    }
    Ok(())
}
impl Service {
    fn edit_authorized(record: &EditRecord, step: &Step) -> bool {
        record.before.mods.iter().filter(|m| !m.retired).any(|m| {
            let Some(g) = record
                .before
                .games
                .iter()
                .find(|g| g.id == m.game_id)
                .and_then(|g| folder_game(g, &m.folder_id).ok())
            else {
                return false;
            };
            m.members.iter().any(|f| {
                (same(&step.from, &f.current_path)
                    && within(&step.to, &g.mod_dir)
                    && !same(&step.to, &g.mod_dir)
                    && step
                        .from
                        .parent()
                        .zip(step.to.parent())
                        .is_some_and(|(a, b)| same(a, b)))
                    || f.leaves.iter().any(|l| {
                        same(&step.from, &l.current)
                            && (same(&l.current, &l.original)
                                || l.disabled_by_manager != Some(false))
                            && (same(&step.to, &l.original)
                                || same(&step.to, &suffix_disabled(&l.original)))
                    })
            })
        })
    }
    fn undo_edit(record: &EditRecord) -> Result<(), String> {
        if record.steps.len() != record.identities.len() {
            return Err("恢复记录不完整".into());
        }
        let mut errors = vec![];
        for (step, identity) in record.steps.iter().zip(&record.identities).rev() {
            let result = (|| {
                if !Self::edit_authorized(record, step) {
                    return Err("恢复记录越界，已阻止文件操作".to_string());
                }
                reject_links(&step.from)?;
                reject_links(&step.to)?;
                if step.to.exists() && !step.from.exists() {
                    if !moved_identity(disk_identity(&step.to), identity) {
                        return Err("恢复文件身份改变，未移动".into());
                    }
                    no_replace_move(&step.to, &step.from)?;
                } else if step.from.exists() && !step.to.exists() {
                    if disk_identity(&step.from).as_ref() != Some(identity) {
                        return Err("源文件身份改变，未恢复".into());
                    }
                } else {
                    return Err("恢复时发现文件冲突或丢失".into());
                }
                Ok(())
            })();
            if let Err(e) = result {
                errors.push(e);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    }
    pub(super) fn mark_pending_edits(&mut self) {
        let Ok(mut q) = self.conn.prepare("SELECT data FROM file_edits") else {
            return;
        };
        let Ok(rows) = q.query_map([], |r| r.get::<_, String>(0)) else {
            return;
        };
        for raw in rows.flatten() {
            if let Ok(record) = serde_json::from_str::<EditRecord>(&raw) {
                for m in &mut self.lib.mods {
                    if record
                        .before
                        .mods
                        .iter()
                        .find(|old| old.id == m.id)
                        .is_some_and(|old| {
                            old.members.iter().any(|f| {
                                record.steps.iter().any(|s| {
                                    same(&s.from, &f.current_path)
                                        || f.leaves.iter().any(|l| same(&s.from, &l.current))
                                })
                            })
                        })
                    {
                        m.state = "error".into();
                    }
                }
            }
        }
    }
    fn restored_library(record: &EditRecord) -> Library {
        let mut lib = record.before.clone();
        for (step, expected) in record.steps.iter().zip(&record.identities) {
            if !moved_identity(disk_identity(&step.from), expected) {
                continue;
            }
            for f in lib.mods.iter_mut().flat_map(|m| m.members.iter_mut()) {
                if same(&f.current_path, &step.from) {
                    f.disk_identity = disk_identity(&step.from);
                }
                for l in &mut f.leaves {
                    if same(&l.current, &step.from) {
                        l.identity = disk_identity(&step.from);
                    }
                }
            }
        }
        lib
    }
    pub(super) fn recover_file_edits(&mut self) -> Result<(), String> {
        let rows = {
            let mut q = self
                .conn
                .prepare("SELECT id,data FROM file_edits")
                .map_err(error)?;
            let rows = q
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(error)?
        };
        for (id, raw) in rows {
            let record: EditRecord = serde_json::from_str(&raw).map_err(error)?;
            validate_library(&record.before)?;
            if record.steps.len() != record.identities.len()
                || record
                    .steps
                    .iter()
                    .zip(&record.identities)
                    .any(|(step, identity)| {
                        !record
                            .before
                            .mods
                            .iter()
                            .filter(|m| !m.retired)
                            .flat_map(|m| m.members.iter())
                            .any(|f| {
                                (same(&f.current_path, &step.from)
                                    && f.disk_identity.as_ref() == Some(identity))
                                    || f.leaves.iter().any(|l| {
                                        same(&l.current, &step.from)
                                            && l.identity.as_ref() == Some(identity)
                                    })
                            })
                    })
            {
                self.warnings
                    .push("恢复记录身份与管理信息不一致，已阻止操作".into());
                continue;
            }
            if record.before.games.iter().any(Self::process_running) {
                self.warnings
                    .push("游戏运行中，未完成改名或启停需退出游戏后重启恢复".into());
                continue;
            }
            match Self::undo_edit(&record) {
                Ok(()) => {
                    self.lib = Self::restored_library(&record);
                    self.persist()?;
                    self.conn
                        .execute("DELETE FROM file_edits WHERE id=?1", params![id])
                        .map_err(error)?;
                }
                Err(e) => self.warnings.push(format!("未完成文件操作需要检查：{e}")),
            }
        }
        Ok(())
    }
    fn perform_edit(
        &mut self,
        steps: Vec<Step>,
        mut next: Library,
        mod_id: &str,
    ) -> Result<(), String> {
        let mut targets = HashSet::new();
        let mut identities = vec![];
        let m = self
            .lib
            .mods
            .iter()
            .find(|m| m.id == mod_id && !m.retired)
            .ok_or("MOD 不存在")?;
        let g = self
            .lib
            .games
            .iter()
            .find(|g| g.id == m.game_id)
            .and_then(|g| folder_game(g, &m.folder_id).ok())
            .ok_or("目录不存在")?;
        // Index only the authorized MOD once, rather than searching the whole
        // library for every file. Actual disk identities are still checked twice.
        let mut sources = HashMap::new();
        for f in &m.members {
            sources.insert(norm(&f.current_path), (f, None));
            for l in &f.leaves {
                sources.insert(norm(&l.current), (f, Some(l)));
            }
        }
        for step in &steps {
            reject_links(&step.from)?;
            reject_links(&step.to)?;
            if !step.from.exists() {
                return Err("源文件缺失".into());
            }
            if !targets.insert(norm(&step.to)) || step.to.exists() {
                return Err(format!("目标已存在，未覆盖：{}", step.to.display()));
            }
            let (f, leaf) = sources
                .get(&norm(&step.from))
                .ok_or("操作文件没有已确认的身份记录")?;
            let authorized = if let Some(l) = leaf {
                (same(&l.current, &l.original) || l.disabled_by_manager != Some(false))
                    && (same(&step.to, &l.original)
                        || same(&step.to, &suffix_disabled(&l.original)))
            } else {
                within(&step.to, &g.mod_dir)
                    && !same(&step.to, &g.mod_dir)
                    && step
                        .from
                        .parent()
                        .zip(step.to.parent())
                        .is_some_and(|(a, b)| same(a, b))
            };
            if !authorized {
                return Err("操作路径超出授权范围".into());
            }
            let expected = leaf
                .map_or_else(|| f.disk_identity.clone(), |l| l.identity.clone())
                .ok_or("操作文件没有已确认的身份记录")?;
            if disk_identity(&step.from).as_ref() != Some(&expected) {
                return Err("文件在检查后被替换，未操作".into());
            }
            identities.push(expected);
        }
        let record = EditRecord {
            before: self.lib.clone(),
            steps,
            identities,
        };
        // Keep all structural/ownership checks, but filesystem link checks only
        // concern the MOD whose paths this operation can actually touch.
        validate_library_for_edit(&next, Some(mod_id))?;
        let id = uid();
        self.conn
            .execute(
                "INSERT INTO file_edits(id,data) VALUES(?1,?2)",
                params![id, serde_json::to_string(&record).map_err(error)?],
            )
            .map_err(error)?;
        for (step, identity) in record.steps.iter().zip(&record.identities) {
            let result = if disk_identity(&step.from).as_ref() != Some(identity) {
                Err("文件在操作前被替换".into())
            } else {
                no_replace_move(&step.from, &step.to)
            };
            if let Err(e) = result {
                match Self::undo_edit(&record) {
                    Ok(()) => {
                        self.lib = Self::restored_library(&record);
                        self.persist()?;
                        self.conn
                            .execute("DELETE FROM file_edits WHERE id=?1", params![id])
                            .map_err(error)?;
                        return Err(format!("操作失败，已恢复原状态：{e}"));
                    }
                    Err(r) => {
                        self.warnings
                            .push(format!("回滚未完成：{r}，请排除占用后重启恢复"));
                        return Err(format!("操作失败，部分文件需恢复：{e}；{r}"));
                    }
                }
            }
            if !moved_identity(disk_identity(&step.to), identity) {
                return Err("操作后文件身份核对失败，已保留恢复记录".into());
            }
        }
        let identities: HashMap<_, _> = record
            .steps
            .iter()
            .map(|step| (norm(&step.to), disk_identity(&step.to)))
            .collect();
        for member in &mut next
            .mods
            .iter_mut()
            .find(|m| m.id == mod_id)
            .ok_or("MOD 不存在")?
            .members
        {
            if let Some(identity) = identities.get(&norm(&member.current_path)) {
                member.disk_identity = identity.clone();
            }
            for leaf in &mut member.leaves {
                if let Some(identity) = identities.get(&norm(&leaf.current)) {
                    leaf.identity = identity.clone();
                }
            }
        }
        let raw = serde_json::to_string(&next).map_err(error)?;
        let transaction = self.conn.transaction().map_err(error)?;
        transaction.execute("INSERT INTO library(id,data) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET data=excluded.data",params![raw]).map_err(error)?;
        transaction
            .execute("DELETE FROM file_edits WHERE id=?1", params![id])
            .map_err(error)?;
        transaction.commit().map_err(error)?;
        self.lib = next;
        self.reconcile_one(mod_id);
        self.persist()?;
        Ok(())
    }
    pub(super) fn toggle_tree(&mut self, m: &Mod, g: &Game, enabled: bool) -> Result<(), String> {
        let mut next = self.lib.clone();
        let edited = next.mods.iter_mut().find(|x| x.id == m.id).unwrap();
        let mut steps = vec![];
        for f in &mut edited.members {
            if f.kind == "directory" {
                if disk_identity(&f.current_path) != f.disk_identity {
                    return Err("文件夹身份改变".into());
                }
                directory_state(f)?;
                if f.leaves.is_empty() && !enabled {
                    return Err("空文件夹没有可禁用的文件".into());
                }
                for l in &mut f.leaves {
                    if !same(&l.current, &l.original) && l.disabled_by_manager == Some(false) {
                        continue;
                    }
                    let to = if enabled {
                        l.original.clone()
                    } else {
                        suffix_disabled(&l.original)
                    };
                    if !same(&l.current, &to) {
                        steps.push(Step {
                            from: l.current.clone(),
                            to: to.clone(),
                        });
                        l.current = to;
                        l.disabled_by_manager = Some(!enabled);
                    }
                }
            } else {
                let to = target(g, f, enabled)?;
                if !same(&f.current_path, &to) {
                    steps.push(Step {
                        from: f.current_path.clone(),
                        to: to.clone(),
                    });
                    f.current_path = to;
                }
            }
        }
        if enabled && steps.is_empty() && m.state == "disabled" {
            return Err(
                "文件夹中的文件原本已禁用，管理器未修改它们；请自行核对后恢复需要的文件".into(),
            );
        }
        // Persist the inventory before recording operations, so recovery can verify every leaf.
        for f in &mut self
            .lib
            .mods
            .iter_mut()
            .find(|x| x.id == m.id)
            .unwrap()
            .members
        {
            if f.kind == "directory" {
                directory_state(f)?;
            }
        }
        self.persist()?;
        self.perform_edit(steps, next, &m.id)?;
        if self.lib.mods.iter().find(|x| x.id == m.id).unwrap().state
            != if enabled { "enabled" } else { "disabled" }
        {
            return Err("操作后文件状态不一致，请刷新核对".into());
        }
        Ok(())
    }
    pub(super) fn rename_member(&mut self, v: &Value) -> Result<(), String> {
        self.ensure_no_journals()?;
        self.reconcile();
        let id = string(v, "id")?;
        let member_id = string(v, "memberId")?;
        let name = string(v, "name")?;
        valid_name(&name)?;
        let m = self
            .lib
            .mods
            .iter()
            .find(|m| m.id == id)
            .ok_or("MOD 不存在")?;
        if m.retired || ["error", "missing"].contains(&m.state.as_str()) {
            return Err("请先解决文件缺失或异常，再修改真实名称".into());
        }
        let g = self
            .lib
            .games
            .iter()
            .find(|g| g.id == m.game_id)
            .and_then(|g| folder_game(g, &m.folder_id).ok())
            .ok_or("目录不存在")?;
        if Self::process_running(&g) {
            return Err("请先退出游戏再修改文件名".into());
        }
        let f = m
            .members
            .iter()
            .find(|f| f.id == member_id)
            .ok_or("成员不存在")?;
        if !within(&f.original_path, &g.mod_dir) || same(&f.original_path, &g.mod_dir) {
            return Err("不能修改公共 MOD 根目录".into());
        }
        if !within(&f.current_path, &g.mod_dir) {
            return Err("请先恢复旧版移出的 MOD，再改名".into());
        }
        if disk_identity(&f.current_path) != f.disk_identity {
            return Err("文件身份改变，未改名".into());
        }
        let to_original = f.original_path.with_file_name(&name);
        if f.kind == "file" && to_original.extension() != f.original_path.extension() {
            return Err("请保留原文件扩展名，只修改名称部分".into());
        }
        if same(&to_original, &f.original_path) {
            if to_original.file_name() != f.original_path.file_name() {
                return Err("仅修改大小写暂不支持，请先改成不同名称，再改为目标名称".into());
            }
            return Ok(());
        }
        if to_original.exists() || suffix_disabled(&to_original).exists() {
            return Err("同名文件或禁用文件已存在，未覆盖".into());
        }
        let to = if f.kind == "file" && !same(&f.current_path, &f.original_path) {
            suffix_disabled(&to_original)
        } else {
            to_original.clone()
        };
        let old = f.original_path.clone();
        let from = f.current_path.clone();
        let mut next = self.lib.clone();
        let edited = next.mods.iter_mut().find(|m| m.id == id).unwrap();
        edited.auto_group = false;
        let f = edited
            .members
            .iter_mut()
            .find(|f| f.id == member_id)
            .unwrap();
        f.original_path = to_original.clone();
        f.current_path = to.clone();
        for l in &mut f.leaves {
            l.original = to_original.join(l.original.strip_prefix(&old).map_err(error)?);
            l.current = to_original.join(l.current.strip_prefix(&old).map_err(error)?);
        }
        self.perform_edit(vec![Step { from, to }], next, &id)
    }
}
