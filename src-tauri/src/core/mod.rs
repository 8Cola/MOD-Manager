use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
mod deletion;
mod drop_import;
mod file_edits;
mod maintenance;
mod restoration;
use file_edits::directory_state;
fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Game {
    id: String,
    name: String,
    mod_dir: PathBuf,
    image_path: Option<PathBuf>,
    exe_path: Option<PathBuf>,
    strategy: String,
    #[serde(default)]
    folders: Vec<ModFolder>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModFolder {
    id: String,
    name: String,
    path: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Member {
    #[serde(default)]
    leaves: Vec<Leaf>,
    id: String,
    original_path: PathBuf,
    current_path: PathBuf,
    kind: String,
    exists: bool,
    #[serde(default)]
    disk_identity: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Leaf {
    original: PathBuf,
    current: PathBuf,
    identity: Option<String>,
    // None is a legacy inventory: preserve its existing restore semantics.
    // New inventories explicitly distinguish manager operations from outside changes.
    #[serde(default)]
    disabled_by_manager: Option<bool>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Mod {
    id: String,
    game_id: String,
    #[serde(default)]
    folder_id: String,
    name: String,
    description: String,
    #[serde(default)]
    image_path: Option<PathBuf>,
    #[serde(default)]
    original_image_path: Option<PathBuf>,
    managed: bool,
    state: String,
    members: Vec<Member>,
    #[serde(default)]
    retired: bool,
    #[serde(default)]
    auto_group: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    theme: String,
    default_strategy: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            default_strategy: "suffix".into(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Default)]
struct Library {
    games: Vec<Game>,
    mods: Vec<Mod>,
    settings: Settings,
}
#[derive(Clone, Serialize, Deserialize)]
struct Step {
    from: PathBuf,
    to: PathBuf,
}
pub struct Service {
    conn: Connection,
    data_dir: PathBuf,
    lib: Library,
    warnings: Vec<String>,
    last_maintenance: Option<std::time::Instant>,
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn string(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("缺少参数：{key}"))
}
fn norm(p: &Path) -> String {
    let text = p.to_string_lossy().replace('/', "\\").to_lowercase();
    if let Some(p) = text.strip_prefix("\\\\?\\unc\\") {
        format!("\\\\{p}")
    } else {
        text.strip_prefix("\\\\?\\").unwrap_or(&text).to_string()
    }
}
pub(crate) fn disk_identity(p: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[repr(C)]
        struct Time {
            low: u32,
            high: u32,
        }
        #[repr(C)]
        struct Info {
            attributes: u32,
            creation: Time,
            access: Time,
            write: Time,
            volume: u32,
            size_high: u32,
            size_low: u32,
            links: u32,
            index_high: u32,
            index_low: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn CreateFileW(
                path: *const u16,
                access: u32,
                share: u32,
                security: *mut std::ffi::c_void,
                creation: u32,
                flags: u32,
                template: *mut std::ffi::c_void,
            ) -> *mut std::ffi::c_void;
            fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Info) -> i32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        let path: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                0,
                7,
                std::ptr::null_mut(),
                3,
                0x02000000,
                std::ptr::null_mut(),
            )
        };
        if handle as isize == -1 {
            return None;
        }
        let mut info = std::mem::MaybeUninit::<Info>::uninit();
        let ok = unsafe { GetFileInformationByHandle(handle, info.as_mut_ptr()) };
        unsafe {
            CloseHandle(handle);
        }
        if ok == 0 {
            return None;
        }
        let info = unsafe { info.assume_init() };
        Some(format!(
            "{}:{}:{}:{}:{}",
            info.volume, info.index_high, info.index_low, info.creation.high, info.creation.low
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(p).ok()?;
        Some(format!("{}:{}", m.dev(), m.ino()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let m = fs::metadata(p).ok()?;
        Some(format!("{:?}", m.created().ok()?))
    }
}
fn same(a: &Path, b: &Path) -> bool {
    norm(a) == norm(b)
}
fn within(p: &Path, root: &Path) -> bool {
    same(p, root) || norm(p).starts_with(&(norm(root) + "\\"))
}
fn reject_links(p: &Path) -> Result<(), String> {
    for ancestor in p.ancestors() {
        if let Ok(m) = fs::symlink_metadata(ancestor) {
            if m.file_type().is_symlink() {
                return Err("为保护文件，无法管理符号链接路径".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    return Err("为保护文件，无法管理目录联接或重解析点".into());
                }
            }
        }
    }
    Ok(())
}
fn reject_tree_links(p: &Path) -> Result<(), String> {
    reject_links(p)?;
    if p.is_dir() {
        for e in fs::read_dir(p).map_err(error)? {
            let p = e.map_err(error)?.path();
            reject_links(&p)?;
            if p.is_dir() {
                reject_tree_links(&p)?;
            }
        }
    }
    Ok(())
}
fn safe_root(p: &Path) -> Result<PathBuf, String> {
    if !p.is_absolute() {
        return Err("请选择绝对文件夹路径".into());
    }
    reject_links(p)?;
    if !p.is_dir() {
        return Err("MOD 文件夹不存在或无法访问".into());
    }
    let p = fs::canonicalize(p).map_err(error)?;
    if p.parent().is_none() {
        return Err("不能将磁盘根目录设为 MOD 文件夹".into());
    }
    Ok(p)
}
fn disabled_root(g: &Game) -> PathBuf {
    let storage_id = g
        .folders
        .iter()
        .find(|f| same(&f.path, &g.mod_dir))
        .map(|f| f.id.as_str())
        .unwrap_or(&g.id);
    g.mod_dir
        .parent()
        .unwrap_or(&g.mod_dir)
        .join(format!(".mod-manager-disabled-{storage_id}"))
}
fn folder_game(g: &Game, folder_id: &str) -> Result<Game, String> {
    let folder = g
        .folders
        .iter()
        .find(|f| f.id == folder_id)
        .ok_or("MOD 所属目录已不存在")?;
    let mut scoped = g.clone();
    scoped.mod_dir = folder.path.clone();
    Ok(scoped)
}
fn normalize_library(lib: &mut Library) {
    for g in &mut lib.games {
        if g.folders.is_empty() {
            g.folders.push(ModFolder {
                id: g.id.clone(),
                name: "MOD".into(),
                path: g.mod_dir.clone(),
            });
        }
    }
    for m in &mut lib.mods {
        if m.folder_id.is_empty() {
            if let Some(g) = lib.games.iter().find(|g| g.id == m.game_id) {
                m.folder_id = g
                    .folders
                    .iter()
                    .find(|f| {
                        m.members
                            .iter()
                            .all(|member| within(&member.original_path, &f.path))
                    })
                    .map(|f| f.id.clone())
                    .unwrap_or_else(|| g.id.clone());
            }
        }
    }
}
fn suffix_original(p: &Path) -> PathBuf {
    let n = p.file_name().unwrap_or_default().to_string_lossy();
    if let Some(n) = n.strip_suffix(".disabled") {
        p.with_file_name(n)
    } else {
        p.to_path_buf()
    }
}
fn suffix_disabled(p: &Path) -> PathBuf {
    let n = p.file_name().unwrap_or_default().to_string_lossy();
    p.with_file_name(format!("{n}.disabled"))
}
fn target(g: &Game, m: &Member, enabled: bool) -> Result<PathBuf, String> {
    if enabled {
        return Ok(m.original_path.clone());
    }
    if g.strategy == "suffix" {
        if m.kind == "directory" {
            return Err("文件夹型 MOD 请使用“移动到禁用目录”策略".into());
        }
        Ok(suffix_disabled(&m.original_path))
    } else {
        let rel = m
            .original_path
            .strip_prefix(&g.mod_dir)
            .map_err(|_| "成员不在游戏 MOD 目录内")?;
        Ok(disabled_root(g).join(rel))
    }
}
fn no_replace_move(from: &Path, to: &Path) -> Result<(), String> {
    reject_links(from)?;
    reject_links(to)?;
    if from.is_dir() {
        reject_tree_links(from)?;
    }
    if fs::symlink_metadata(to).is_ok() {
        return Err(format!("目标已存在，未覆盖：{}", to.display()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> i32;
        }
        let a: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 8) } == 0 {
            return Err(format!(
                "文件移动失败（权限、占用或冲突）：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(from, to).map_err(error)
    }
}
impl Service {
    pub(crate) fn monitor_configuration(&self) -> Value {
        // Watch registration only needs directory configuration, not a disk
        // reconciliation of every MOD and a serialization of every leaf.
        json!({"games":self.lib.games})
    }
    fn ensure_no_journals(&self) -> Result<(), String> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM journals) + (SELECT COUNT(*) FROM file_edits)",
                [],
                |r| r.get(0),
            )
            .map_err(error)?;
        if count > 0 {
            Err("存在未完成文件操作，请先排除占用或权限问题并重启恢复，再修改配置或导入".into())
        } else {
            Ok(())
        }
    }
    pub fn new(data_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&data_dir).map_err(error)?;
        reject_links(&data_dir)?;
        let conn = Connection::open(data_dir.join("manager.sqlite3")).map_err(error)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS library(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS journals(id TEXT PRIMARY KEY,plan TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'pending'); CREATE TABLE IF NOT EXISTS history(id INTEGER PRIMARY KEY,at TEXT DEFAULT CURRENT_TIMESTAMP,mod_id TEXT,enabled INTEGER,result TEXT);").map_err(error)?;
        let _ = conn.execute(
            "ALTER TABLE journals ADD COLUMN status TEXT NOT NULL DEFAULT 'pending'",
            [],
        );
        conn.execute_batch("CREATE TABLE IF NOT EXISTS deletions(id TEXT PRIMARY KEY,data TEXT NOT NULL,status TEXT NOT NULL,at TEXT DEFAULT CURRENT_TIMESTAMP);").map_err(error)?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS preferences(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL);").map_err(error)?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS file_imports(id TEXT PRIMARY KEY,source TEXT NOT NULL,target TEXT NOT NULL,stage TEXT NOT NULL,status TEXT NOT NULL,detail TEXT,at TEXT DEFAULT CURRENT_TIMESTAMP);").map_err(error)?;
        maintenance::migrate(&conn)?;
        let raw = conn
            .query_row("SELECT data FROM library WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .ok();
        let mut lib = match raw {
            Some(s) => serde_json::from_str(&s).map_err(error)?,
            None => Library::default(),
        };
        normalize_library(&mut lib);
        validate_library(&lib)?;
        let mut s = Self {
            conn,
            data_dir,
            lib,
            warnings: vec![],
            last_maintenance: None,
        };
        s.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS file_edits(id TEXT PRIMARY KEY,data TEXT NOT NULL);",
            )
            .map_err(error)?;
        s.recover_file_edits()?;
        s.recover()?;
        s.recover_deletions()?;
        s.check_interrupted_imports()?;
        s.scan(None)?;
        if let Err(e) = s.maintain_storage() {
            s.warnings.push(format!("存储整理暂未完成：{e}"));
        }
        Ok(s)
    }
    fn authorized_step(&self, step: &Step) -> bool {
        self.lib.mods.iter().filter(|m| !m.retired).any(|m| {
            self.lib
                .games
                .iter()
                .find(|g| g.id == m.game_id)
                .and_then(|g| folder_game(g, &m.folder_id).ok())
                .is_some_and(|g| {
                    m.members.iter().any(|f| {
                        let off = target(&g, f, false).ok();
                        off.is_some_and(|off| {
                            (same(&step.from, &f.original_path) && same(&step.to, &off))
                                || (same(&step.to, &f.original_path) && same(&step.from, &off))
                        })
                    })
                })
        })
    }
    fn persist(&mut self) -> Result<(), String> {
        let raw = serde_json::to_string(&self.lib).map_err(error)?;
        self.conn.execute("INSERT INTO library(id,data) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET data=excluded.data",params![raw]).map_err(error)?;
        Ok(())
    }
    fn recover(&mut self) -> Result<(), String> {
        let rows = {
            let mut q = self
                .conn
                .prepare("SELECT id,plan FROM journals")
                .map_err(error)?;
            let rs = q
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(error)?;
            rs.collect::<Result<Vec<_>, _>>().map_err(error)?
        };
        for (id, p) in rows {
            let plan: Vec<Step> = serde_json::from_str(&p).map_err(error)?;
            if plan.iter().any(|step| !self.authorized_step(step)) {
                self.warnings
                    .push("发现不属于已配置 MOD 的恢复记录，已阻止自动文件操作".into());
                continue;
            }
            if self.lib.games.iter().any(|g| {
                Self::process_running(g)
                    && self.lib.mods.iter().filter(|m| m.game_id == g.id).any(|m| {
                        m.members.iter().any(|f| {
                            plan.iter().any(|p| {
                                same(&p.from, &f.original_path) || same(&p.to, &f.original_path)
                            })
                        })
                    })
            }) {
                self.conn
                    .execute(
                        "UPDATE journals SET status='error' WHERE id=?1",
                        params![id],
                    )
                    .map_err(error)?;
                self.warnings
                    .push("游戏正在运行，已暂停未完成操作恢复；退出游戏后重启管理器以恢复".into());
                continue;
            }
            let mut failed = false;
            for step in plan.iter().rev() {
                if step.to.exists() && !step.from.exists() {
                    let expected = self
                        .lib
                        .mods
                        .iter()
                        .filter(|m| !m.retired)
                        .flat_map(|m| m.members.iter())
                        .find(|f| {
                            same(&f.original_path, &step.from) || same(&f.original_path, &step.to)
                        })
                        .and_then(|f| f.disk_identity.as_ref());
                    if expected.is_none() || disk_identity(&step.to).as_ref() != expected {
                        failed = true;
                        self.warnings.push(format!(
                            "恢复成员身份无法确认，未移动该文件：{}",
                            step.to.display()
                        ));
                        continue;
                    }
                    if let Err(e) = no_replace_move(&step.to, &step.from) {
                        failed = true;
                        self.warnings.push(format!("未完成操作无法恢复：{e}"));
                    }
                } else if step.to.exists() && step.from.exists() {
                    failed = true;
                    self.warnings
                        .push(format!("恢复操作发现路径冲突：{}", step.from.display()));
                } else if !step.to.exists() && !step.from.exists() {
                    failed = true;
                    self.warnings
                        .push(format!("恢复操作的成员已缺失：{}", step.from.display()));
                }
            }
            if !failed {
                self.conn
                    .execute("DELETE FROM journals WHERE id=?1", params![id])
                    .map_err(error)?;
            } else {
                self.conn
                    .execute(
                        "UPDATE journals SET status='error' WHERE id=?1",
                        params![id],
                    )
                    .map_err(error)?;
            }
        }
        Ok(())
    }
    fn reconcile(&mut self) {
        self.reconcile_selected(None);
    }
    fn reconcile_one(&mut self, id: &str) {
        self.reconcile_selected(Some(&HashSet::from([id.to_string()])));
    }
    fn reconcile_selected(&mut self, ids: Option<&HashSet<String>>) {
        for m in &mut self.lib.mods {
            if ids.is_some_and(|ids| !ids.contains(&m.id)) {
                continue;
            }
            if m.retired {
                m.state = "missing".into();
                m.managed = false;
                for f in &mut m.members {
                    f.exists = false;
                }
                continue;
            }
            let Some(g) = self
                .lib
                .games
                .iter()
                .find(|g| g.id == m.game_id)
                .and_then(|g| folder_game(g, &m.folder_id).ok())
            else {
                continue;
            };
            let mut on = 0;
            let mut off = 0;
            let mut missing = 0;
            let mut conflict = false;
            for f in &mut m.members {
                if g.strategy == "suffix" && f.kind == "directory" && f.original_path.exists() {
                    f.current_path = f.original_path.clone();
                    f.exists = true;
                    if disk_identity(&f.current_path) != f.disk_identity {
                        conflict = true;
                        continue;
                    }
                    match directory_state(f) {
                        Ok((a, b, c)) => {
                            on += a;
                            off += b;
                            missing += c;
                        }
                        Err(_) => conflict = true,
                    }
                    continue;
                }
                let a = f.original_path.clone();
                let b = if g.strategy == "suffix" && f.kind != "directory" {
                    suffix_disabled(&a)
                } else {
                    disabled_root(&g).join(a.strip_prefix(&g.mod_dir).unwrap_or(&a))
                };
                let ea = a.exists();
                let eb = b.exists();
                if ea && eb {
                    conflict = true;
                    f.exists = true;
                } else if ea {
                    f.current_path = a;
                    f.exists = true;
                    on += 1;
                } else if eb {
                    f.current_path = b;
                    f.exists = true;
                    off += 1;
                } else {
                    f.exists = false;
                    missing += 1;
                }
                if f.exists && !(ea && eb) {
                    let actual = disk_identity(&f.current_path);
                    if f.disk_identity.is_none() && actual.is_some() {
                        f.disk_identity = actual.clone();
                    }
                    if actual.is_none() || actual != f.disk_identity {
                        conflict = true;
                        let warning = format!(
                            "文件身份改变或无法核对，已阻止自动关联与启停：{}",
                            f.current_path.display()
                        );
                        if !self.warnings.contains(&warning) {
                            self.warnings.push(warning);
                        }
                    }
                }
            }
            m.state = if conflict {
                "error"
            } else if missing > 0 {
                "missing"
            } else if on > 0 && off > 0 {
                "mixed"
            } else if off > 0 {
                "disabled"
            } else {
                "enabled"
            }
            .into();
        }
        for raw in self.failed_plans() {
            if let Ok(plan) = serde_json::from_str::<Vec<Step>>(&raw) {
                for m in &mut self.lib.mods {
                    if m.members.iter().any(|f| {
                        plan.iter().any(|p| {
                            same(&f.original_path, &p.from) || same(&f.original_path, &p.to)
                        })
                    }) {
                        m.state = "error".into();
                    }
                }
            }
        }
        self.mark_pending_edits();
    }
    fn failed_plans(&self) -> Vec<String> {
        let Ok(mut q) = self
            .conn
            .prepare("SELECT plan FROM journals WHERE status='error'")
        else {
            return vec![];
        };
        let Ok(rows) = q.query_map([], |r| r.get::<_, String>(0)) else {
            return vec![];
        };
        rows.filter_map(Result::ok).collect()
    }
    fn snapshot(&mut self) -> Value {
        self.reconcile();
        let mods: Vec<Value> = self.lib.mods.iter().map(Self::mod_value).collect();
        let view: Value = self
            .conn
            .query_row("SELECT data FROM preferences WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(json!({}));
        json!({"games":self.lib.games,"mods":mods,"settings":self.lib.settings,"view":view,"recentDeletions":self.recent_deletions(),"pendingImports":self.pending_imports(),"dataDir":self.data_dir,"monitoring":"正在监控","warnings":self.warnings})
    }
    fn mod_value(m: &Mod) -> Value {
        let mut value = serde_json::to_value(m).expect("Mod metadata serializes");
        value["preservedDisabledFiles"] = json!(m
            .members
            .iter()
            .flat_map(|f| &f.leaves)
            .filter(|l| l.disabled_by_manager == Some(false)
                && !same(&l.current, &l.original)
                && l.current.exists())
            .count());
        value["identityChanged"] = json!(
            !m.retired
                && m.members.iter().any(|f| f.exists
                    && (disk_identity(&f.current_path) != f.disk_identity
                        || f.leaves.iter().any(
                            |l| l.current.exists() && disk_identity(&l.current) != l.identity
                        )))
        );
        if let Some(members) = value["members"].as_array_mut() {
            for member in members {
                if let Some(leaves) = member["leaves"].as_array_mut() {
                    for leaf in leaves {
                        leaf["exists"] = json!(leaf["current"]
                            .as_str()
                            .is_some_and(|p| Path::new(p).exists()));
                    }
                }
            }
        }
        value
    }
    fn accept_replacement(&mut self, v: &Value) -> Result<(), String> {
        if v.get("confirmed").and_then(Value::as_bool) != Some(true) {
            return Err("请明确确认新文件继续使用此 MOD 的名称和备注".into());
        }
        self.ensure_no_journals()?;
        let id = string(v, "id")?;
        let index = self
            .lib
            .mods
            .iter()
            .position(|m| m.id == id)
            .ok_or("MOD 不存在")?;
        let m = &self.lib.mods[index];
        if m.retired {
            return Err("旧目录归档记录不能重新关联文件".into());
        }
        let g = self
            .lib
            .games
            .iter()
            .find(|g| g.id == m.game_id)
            .and_then(|g| folder_game(g, &m.folder_id).ok())
            .ok_or("游戏不存在")?;
        reject_links(&g.mod_dir)?;
        reject_links(&disabled_root(&g))?;
        let mut identities = Vec::new();
        let mut resolved_paths = Vec::new();
        for f in &m.members {
            if !f.original_path.is_absolute()
                || !within(&f.original_path, &g.mod_dir)
                || same(&f.original_path, &g.mod_dir)
                || f.original_path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err("成员路径超出授权 MOD 目录".into());
            }
            let off = if g.strategy == "suffix" && f.kind == "directory" {
                f.original_path.clone()
            } else if g.strategy == "suffix" && f.kind == "file" {
                suffix_disabled(&f.original_path)
            } else {
                disabled_root(&g).join(f.original_path.strip_prefix(&g.mod_dir).map_err(error)?)
            };
            reject_links(&f.original_path)?;
            reject_links(&off)?;
            let original = f.original_path.try_exists().map_err(error)?;
            let disabled = off.try_exists().map_err(error)?;
            if original == disabled
                && !(g.strategy == "suffix" && f.kind == "directory" && original)
            {
                return Err(if original {
                    "原始与禁用路径同时存在，请先解决文件冲突"
                } else {
                    "成员文件缺失，无法确认替换文件"
                }
                .into());
            }
            let path = if original {
                f.original_path.clone()
            } else {
                off
            };
            let metadata = fs::symlink_metadata(&path).map_err(error)?;
            if (f.kind == "file" && !metadata.is_file())
                || (f.kind == "directory" && !metadata.is_dir())
                || !["file", "directory"].contains(&f.kind.as_str())
            {
                return Err("替换文件类型与原成员不一致".into());
            }
            if f.kind == "directory" {
                reject_tree_links(&path)?;
            }
            let identity =
                disk_identity(&path).ok_or("无法核对替换文件的磁盘身份，请检查文件权限与占用")?;
            identities.push(Some(identity));
            resolved_paths.push(path);
        }
        for (f, identity) in self.lib.mods[index].members.iter_mut().zip(identities) {
            f.disk_identity = identity;
            if f.kind == "directory" {
                f.leaves.clear();
                directory_state(f)?;
            }
        }
        for path in resolved_paths {
            let warning = format!(
                "文件身份改变或无法核对，已阻止自动关联与启停：{}",
                path.display()
            );
            self.warnings.retain(|w| w != &warning);
        }
        self.reconcile();
        self.persist()
    }
    fn scan(&mut self, game_id: Option<&str>) -> Result<(), String> {
        if self
            .conn
            .query_row("SELECT COUNT(*) FROM file_edits", [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(error)?
            > 0
        {
            self.reconcile();
            return self.persist();
        }
        let games: Vec<_> = self
            .lib
            .games
            .iter()
            .flat_map(|g| g.folders.iter().filter_map(|f| folder_game(g, &f.id).ok()))
            .collect();
        for g in games {
            if game_id.is_some_and(|id| id != g.id) {
                continue;
            }
            if let Err(e) = reject_links(&g.mod_dir) {
                self.warnings.push(format!("{}：{e}", g.name));
                continue;
            }
            let entries = match fs::read_dir(&g.mod_dir) {
                Ok(x) => x,
                Err(e) => {
                    let w = format!("{}：无法读取 MOD 目录（{e}）", g.name);
                    if !self.warnings.contains(&w) {
                        self.warnings.push(w);
                    }
                    continue;
                }
            };
            let folder_id = g
                .folders
                .iter()
                .find(|f| same(&f.path, &g.mod_dir))
                .unwrap()
                .id
                .clone();
            let known: HashSet<String> = self
                .lib
                .mods
                .iter()
                .filter(|m| m.game_id == g.id && m.folder_id == folder_id && !m.retired)
                .flat_map(|m| m.members.iter().map(|f| norm(&f.original_path)))
                .collect();
            let mut found: HashMap<String, Vec<Member>> = HashMap::new();
            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let p = entry.path();
                if reject_links(&p).is_err() {
                    let w = format!("已忽略符号链接或目录联接：{}", p.display());
                    if !self.warnings.contains(&w) {
                        self.warnings.push(w);
                    }
                    continue;
                }
                let meta = match fs::symlink_metadata(&p) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if !meta.is_file() && !meta.is_dir() {
                    continue;
                }
                let original = if meta.is_file() {
                    suffix_original(&p)
                } else {
                    p.clone()
                };
                if known.contains(&norm(&original)) {
                    continue;
                }
                let ext = original
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                let key = if meta.is_file() && ["pak", "utoc", "ucas"].contains(&ext.as_str()) {
                    format!("triple:{}", norm(&original.with_extension("")))
                } else {
                    norm(&original)
                };
                // A counterpart collision is represented once and reconciled as an error.
                let list = found.entry(key).or_default();
                if !list.iter().any(|f| same(&f.original_path, &original)) {
                    list.push(Member {
                        leaves: vec![],
                        disk_identity: disk_identity(&p),
                        id: uid(),
                        original_path: original,
                        current_path: p,
                        kind: if meta.is_dir() { "directory" } else { "file" }.into(),
                        exists: true,
                    });
                }
            }
            let mut found: Vec<_> = found.into_values().collect();
            found.sort_by_key(|v| norm(&v[0].original_path));
            for mut members in found {
                members.sort_by_key(|f| norm(&f.original_path));
                let managed = members.iter().all(|f| {
                    f.kind == "file"
                        && ["pak", "utoc", "ucas"].contains(
                            &f.original_path
                                .extension()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_lowercase()
                                .as_str(),
                        )
                });
                if managed {
                    if let Some(existing) = self.lib.mods.iter_mut().find(|m| {
                        m.game_id == g.id
                            && m.folder_id == folder_id
                            && m.auto_group
                            && !m.retired
                            && m.members.iter().all(|f| {
                                same(
                                    &f.original_path.with_extension(""),
                                    &members[0].original_path.with_extension(""),
                                )
                            })
                    }) {
                        existing.members.extend(members);
                        continue;
                    }
                }
                self.lib.mods.push(Mod {
                    id: uid(),
                    game_id: g.id.clone(),
                    folder_id: folder_id.clone(),
                    name: String::new(),
                    description: String::new(),
                    image_path: None,
                    original_image_path: None,
                    managed,
                    state: "enabled".into(),
                    members,
                    retired: false,
                    auto_group: managed,
                });
            }
        }
        self.reconcile();
        self.persist()
    }
    fn save_game(&mut self, v: &Value) -> Result<(), String> {
        self.ensure_no_journals()?;
        self.reconcile();
        let name = string(v, "name")?.trim().to_string();
        if name.is_empty() {
            return Err("请输入游戏名称".into());
        }
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(uid);
        if v.get("id").is_some_and(|id| !id.is_null())
            && (uuid::Uuid::parse_str(&id).is_err() || !self.lib.games.iter().any(|g| g.id == id))
        {
            return Err("游戏标识无效，新增游戏的标识由管理器生成".into());
        }
        let old = self.lib.games.iter().find(|g| g.id == id).cloned();
        let strategy = v
            .get("strategy")
            .and_then(Value::as_str)
            .unwrap_or(&self.lib.settings.default_strategy)
            .to_string();
        if !["suffix", "move"].contains(&strategy.as_str()) {
            return Err("未知禁用策略".into());
        }
        let raw_folders = match v.get("folders") {
            Some(Value::Array(rows)) => rows.clone(),
            Some(_) => return Err("目录列表格式无效".into()),
            None => vec![
                json!({"id":old.as_ref().and_then(|g|g.folders.first()).map(|f|&f.id),"name":"MOD","path":string(v,"modDir")?}),
            ],
        };
        if raw_folders.is_empty() || raw_folders.len() > 32 {
            return Err("请配置 1 至 32 个独立 MOD 目录".into());
        }
        let mut folders = Vec::new();
        let mut seen = HashSet::new();
        for (index, row) in raw_folders.iter().enumerate() {
            let path = safe_root(&PathBuf::from(string(row, "path")?))?;
            let folder_name = string(row, "name")?.trim().to_string();
            if folder_name.is_empty() || folder_name.chars().count() > 80 {
                return Err("请为每个目录填写不超过 80 字的备注名".into());
            }
            let folder_id = match row
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            {
                Some(fid)
                    if old
                        .as_ref()
                        .is_some_and(|g| g.folders.iter().any(|f| f.id == fid)) =>
                {
                    fid.to_string()
                }
                Some(_) => return Err("目录标识无效，请重新添加目录".into()),
                None if old.is_none() && index == 0 => id.clone(),
                None => uid(),
            };
            if !seen.insert(folder_id.clone()) {
                return Err("目录标识重复".into());
            }
            folders.push(ModFolder {
                id: folder_id,
                name: folder_name,
                path,
            });
        }
        let mut candidate = self.lib.clone();
        candidate.games.retain(|g| g.id != id);
        let probe = Game {
            id: id.clone(),
            name: name.clone(),
            mod_dir: folders[0].path.clone(),
            folders: folders.clone(),
            image_path: None,
            exe_path: None,
            strategy: strategy.clone(),
        };
        candidate.games.push(probe);
        validate_roots(&candidate.games)?;
        if let Some(old) = &old {
            for previous in &old.folders {
                let changed = !folders
                    .iter()
                    .any(|f| f.id == previous.id && same(&f.path, &previous.path));
                if changed {
                    if self.lib.mods.iter().any(|m| {
                        m.game_id == id
                            && m.folder_id == previous.id
                            && !m.retired
                            && ((m.state != "enabled" && m.state != "missing")
                                || m.members.iter().any(|f| {
                                    f.current_path.exists()
                                        && !same(&f.original_path, &f.current_path)
                                }))
                    }) {
                        return Err(format!("请先开启「{}」目录中的全部 MOD，再修改或移除该目录，确保禁用文件可以恢复",previous.name));
                    }
                    for m in &mut self.lib.mods {
                        if m.game_id == id && m.folder_id == previous.id {
                            m.retired = true;
                            m.managed = false;
                        }
                    }
                    self.warnings.push(format!(
                        "「{}」目录已修改或移除；旧名称和备注保留为历史记录，真实文件未改动",
                        previous.name
                    ));
                }
            }
            if old.strategy != strategy
                && self.lib.mods.iter().any(|m| {
                    m.game_id == id
                        && !m.retired
                        && ((m.state != "enabled" && m.state != "missing")
                            || m.members.iter().any(|f| {
                                f.current_path.exists() && !same(&f.original_path, &f.current_path)
                            }))
                })
            {
                return Err("请先开启所有目录中的全部 MOD，再更换禁用策略".into());
            }
        }
        let mod_dir = folders[0].path.clone();
        let exe_path = v
            .get("exePath")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        if let Some(p) = &exe_path {
            if !p.is_absolute()
                || !p
                    .extension()
                    .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("exe"))
            {
                return Err("请选择有效的绝对游戏 EXE 路径".into());
            }
            reject_links(p)?;
            if !p.is_file() {
                return Err("游戏 EXE 路径不存在".into());
            }
        }
        let exe_path = exe_path
            .map(|p| fs::canonicalize(p).map_err(error))
            .transpose()?;
        let image_path = match v.get("imagePath") {
            Some(Value::String(p)) if !p.is_empty() => {
                Some(self.copy_image(&PathBuf::from(p), &id)?)
            }
            Some(Value::Null) => None,
            _ => old.as_ref().and_then(|g| g.image_path.clone()),
        };
        let image_path = if image_path.is_none() && !v.get("imagePath").is_some_and(Value::is_null)
        {
            exe_path.as_ref().and_then(|p| {
                self.local_image(p, &id).or_else(|| {
                    crate::assets::local_game_image(p, &self.data_dir.join("images"))
                        .and_then(|image| self.copy_image(&image, &id).ok())
                })
            })
        } else {
            image_path
        };
        let g = Game {
            id: id.clone(),
            name,
            mod_dir,
            folders,
            image_path,
            exe_path,
            strategy,
        };
        if let Some(i) = self.lib.games.iter().position(|g| g.id == id) {
            self.lib.games[i] = g;
        } else {
            self.lib.games.push(g);
        }
        self.scan(Some(&id))
    }
    fn copy_image(&self, p: &Path, id: &str) -> Result<PathBuf, String> {
        reject_links(p)?;
        let ext = p
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if !["png", "jpg", "jpeg", "webp"].contains(&ext.as_str()) {
            return Err("图片支持 PNG、JPG 和 WebP".into());
        }
        let bytes = fs::read(p).map_err(error)?;
        if bytes.len() > 20 * 1024 * 1024 {
            return Err("图片不能超过 20 MB".into());
        }
        validate_image(&bytes, &ext)?;
        let dir = self.data_dir.join("images");
        fs::create_dir_all(&dir).map_err(error)?;
        reject_links(&dir)?;
        let dest = dir.join(format!("{}-{}.{}", id, uid(), ext));
        fs::write(&dest, bytes).map_err(error)?;
        Ok(dest)
    }
    fn local_image(&self, exe: &Path, id: &str) -> Option<PathBuf> {
        // Steam local app manifest supplies app ID; no network access.
        let common = exe
            .ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == "common"))?;
        let steamapps = common.parent()?;
        let install = exe
            .strip_prefix(common)
            .ok()?
            .components()
            .next()?
            .as_os_str()
            .to_string_lossy();
        for f in fs::read_dir(steamapps).ok()?.flatten() {
            let n = f.file_name().to_string_lossy().into_owned();
            if !n.starts_with("appmanifest_") || !n.ends_with(".acf") {
                continue;
            }
            let text = fs::read_to_string(f.path()).ok()?;
            if !text.contains(&format!("\"{install}\"")) {
                continue;
            }
            let appid = n
                .trim_start_matches("appmanifest_")
                .trim_end_matches(".acf");
            let steam = steamapps.parent()?;
            for name in [
                format!("{appid}_icon.jpg"),
                format!("{appid}_header.jpg"),
                format!("{appid}_library_600x900.jpg"),
            ] {
                let p = steam.join("appcache/librarycache").join(name);
                if p.is_file() {
                    if let Ok(p) = self.copy_image(&p, id) {
                        return Some(p);
                    }
                }
            }
        }
        None
    }
    fn edit_mod(&mut self, v: &Value) -> Result<(), String> {
        let id = string(v, "id")?;
        if !self.lib.mods.iter().any(|m| m.id == id) {
            return Err("MOD 不存在".into());
        }
        let image = match v.get("imagePath") {
            Some(Value::Null) => Some(None),
            Some(Value::String(path)) => Some(Some(self.copy_image(Path::new(path), &id)?)),
            Some(_) => return Err("图片路径无效".into()),
            None => None,
        };
        let original = if image.is_some() {
            match v.get("originalImagePath") {
                Some(Value::String(path)) if !v["imagePath"].is_null() => {
                    Some(self.copy_image(Path::new(path), &format!("original-{id}"))?)
                }
                Some(Value::Null) | None => None,
                _ => return Err("原图路径无效".into()),
            }
        } else {
            if v.get("originalImagePath").is_some() {
                return Err("更换原图时需同时设置封面".into());
            }
            None
        };
        let m = self
            .lib
            .mods
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or("MOD 不存在")?;
        if let Some(n) = v.get("name").and_then(Value::as_str) {
            m.name = n.into();
        }
        if let Some(d) = v.get("description").and_then(Value::as_str) {
            m.description = d.into();
        }
        if let Some(image) = image {
            m.image_path = image;
            m.original_image_path = original;
        }
        if let Some(a) = v.get("managed").and_then(Value::as_bool) {
            if a && m.retired {
                return Err("该记录属于此前目录，仅保留名称和备注，不能重新纳入启停管理".into());
            }
            m.managed = a;
        }
        self.persist()
    }
    fn process_running(g: &Game) -> bool {
        let Some(exe) = &g.exe_path else { return false };
        let exe = fs::canonicalize(exe).unwrap_or_else(|_| exe.clone());
        // Only process names and executable paths are needed. CPU/memory/device
        // collection is expensive on Windows and irrelevant to this safety check.
        let sys =
            sysinfo::System::new_with_specifics(sysinfo::RefreshKind::nothing().with_processes(
                sysinfo::ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always),
            ));
        sys.processes().values().any(|p| {
            p.exe().map_or_else(
                || {
                    exe.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .eq_ignore_ascii_case(&p.name().to_string_lossy())
                    })
                },
                |path| same(path, &exe),
            )
        })
    }
    fn toggle_one(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        self.ensure_no_journals()?;
        self.reconcile_one(id);
        let m = self
            .lib
            .mods
            .iter()
            .find(|m| m.id == id)
            .ok_or("MOD 不存在")?
            .clone();
        if m.retired {
            return Err("该记录属于此前的 MOD 目录，保留备注但不能操作".into());
        }
        if !m.managed {
            return Err("请先在详情中确认将此文件纳入管理".into());
        }
        let g = self
            .lib
            .games
            .iter()
            .find(|g| g.id == m.game_id)
            .and_then(|g| folder_game(g, &m.folder_id).ok())
            .ok_or("游戏不存在")?;
        if Self::process_running(&g) {
            return Err("游戏正在运行，请退出游戏后再操作 MOD".into());
        }
        reject_links(&g.mod_dir)?;
        reject_links(&disabled_root(&g))?;
        if ["missing", "error"].contains(&m.state.as_str()) {
            return Err("成员文件缺失或存在路径冲突，请先核对详情并刷新".into());
        }
        if g.strategy == "suffix" && m.members.iter().any(|f| f.kind == "directory") {
            return self.toggle_tree(&m, &g, enabled);
        }
        let mut plan = vec![];
        let mut targets = HashSet::new();
        for f in &m.members {
            if !within(&f.original_path, &g.mod_dir) || same(&f.original_path, &g.mod_dir) {
                return Err("成员路径超出授权 MOD 目录".into());
            }
            let to = target(&g, f, enabled)?;
            let from = f.current_path.clone();
            if !within(&from, &g.mod_dir) && !within(&from, &disabled_root(&g)) {
                return Err("当前成员路径超出授权目录".into());
            }
            reject_links(&from)?;
            reject_links(&to)?;
            if f.kind == "directory" {
                reject_tree_links(&from)?;
            }
            if !from.exists() {
                return Err("成员文件已丢失".into());
            }
            if !targets.insert(norm(&to)) {
                return Err("成员目标路径重复".into());
            }
            if same(&from, &to) {
                continue;
            }
            if fs::symlink_metadata(&to).is_ok() {
                return Err(format!("目标已存在，未覆盖：{}", to.display()));
            }
            plan.push(Step { from, to });
        }
        if plan.is_empty() {
            return Ok(());
        }
        for step in &plan {
            let parent = step.to.parent().ok_or("目标目录无效")?;
            if !parent.exists() {
                if !within(parent, &disabled_root(&g)) {
                    return Err("原始 MOD 父目录不存在，请手动恢复目录".into());
                }
                fs::create_dir_all(parent).map_err(error)?;
                reject_links(parent)?;
            }
        }
        let journal = uid();
        self.conn
            .execute(
                "INSERT INTO journals(id,plan) VALUES(?1,?2)",
                params![journal, serde_json::to_string(&plan).map_err(error)?],
            )
            .map_err(error)?;
        let mut done: Vec<Step> = vec![];
        for step in &plan {
            let expected = m
                .members
                .iter()
                .find(|f| same(&f.current_path, &step.from))
                .and_then(|f| f.disk_identity.as_ref());
            let movement = if expected.is_none() || disk_identity(&step.from).as_ref() != expected {
                Err("成员文件在操作前被替换或无法核对身份，未移动".to_string())
            } else {
                no_replace_move(&step.from, &step.to)
            };
            if let Err(e) = movement {
                let mut failures = vec![];
                for d in done.iter().rev() {
                    let expected = m
                        .members
                        .iter()
                        .find(|f| same(&f.current_path, &d.from))
                        .and_then(|f| f.disk_identity.as_ref());
                    if expected.is_none() || disk_identity(&d.to).as_ref() != expected {
                        failures.push("回滚成员身份改变，未移动该成员".into());
                        continue;
                    }
                    if let Err(r) = no_replace_move(&d.to, &d.from) {
                        failures.push(r);
                    }
                }
                self.reconcile_one(id);
                self.persist()?;
                if failures.is_empty() {
                    self.conn
                        .execute("DELETE FROM journals WHERE id=?1", params![journal])
                        .map_err(error)?;
                    return Err(format!("操作未完成，已恢复原状态：{e}"));
                }
                if let Some(m) = self.lib.mods.iter_mut().find(|m| m.id == id) {
                    m.state = "error".into();
                }
                self.conn
                    .execute(
                        "UPDATE journals SET status='error' WHERE id=?1",
                        params![journal],
                    )
                    .map_err(error)?;
                self.warnings.push(
                    "部分成员无法恢复：请排除文件占用或权限问题后重启软件，恢复未完成操作".into(),
                );
                self.persist()?;
                return Err(format!(
                    "操作失败且部分文件无法恢复：{e}；{}",
                    failures.join("；")
                ));
            }
            done.push(step.clone());
        }
        for f in &mut self
            .lib
            .mods
            .iter_mut()
            .find(|m| m.id == id)
            .unwrap()
            .members
        {
            f.current_path = target(&g, f, enabled)?;
        }
        self.reconcile_one(id);
        let state = &self.lib.mods.iter().find(|m| m.id == id).unwrap().state;
        if state != if enabled { "enabled" } else { "disabled" } {
            self.persist()?;
            return Err("操作后状态核对失败；请查看成员文件并刷新".into());
        }
        self.persist()?;
        self.conn
            .execute("DELETE FROM journals WHERE id=?1", params![journal])
            .map_err(error)?;
        Ok(())
    }
    fn grouping(&mut self, v: &Value) -> Result<(), String> {
        self.ensure_no_journals()?;
        let ids = parse_ids(v, "ids")?;
        if ids.len() < 2 {
            return Err("请选择至少两个 MOD".into());
        }
        let chosen: Vec<Mod> = ids
            .iter()
            .map(|id| {
                self.lib
                    .mods
                    .iter()
                    .find(|m| &m.id == id)
                    .cloned()
                    .ok_or("选择的 MOD 已不存在")
            })
            .collect::<Result<_, _>>()?;
        let gid = &chosen[0].game_id;
        if chosen.iter().any(|m| m.retired) {
            return Err("旧目录记录不能重新分组".into());
        }
        if chosen
            .iter()
            .any(|m| &m.game_id != gid || m.folder_id != chosen[0].folder_id)
        {
            return Err("只能合并同一游戏、同一目录的 MOD".into());
        }
        let m = Mod {
            id: uid(),
            game_id: gid.clone(),
            folder_id: chosen[0].folder_id.clone(),
            name: v.get("name").and_then(Value::as_str).unwrap_or("").into(),
            description: v
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            image_path: chosen[0].image_path.clone(),
            original_image_path: chosen[0].original_image_path.clone(),
            managed: chosen.iter().all(|m| m.managed),
            state: "mixed".into(),
            members: chosen.into_iter().flat_map(|m| m.members).collect(),
            retired: false,
            auto_group: false,
        };
        self.lib.mods.retain(|m| !ids.contains(&m.id));
        self.lib.mods.push(m);
        self.reconcile();
        self.persist()
    }
    fn members(&mut self, v: &Value, ungroup: bool) -> Result<(), String> {
        self.ensure_no_journals()?;
        let id = string(v, "id")?;
        let index = self
            .lib
            .mods
            .iter()
            .position(|m| m.id == id)
            .ok_or("MOD 不存在")?;
        let source = self.lib.mods[index].clone();
        if source.retired {
            return Err("旧目录记录不能调整成员".into());
        }
        let wanted = if ungroup {
            vec![]
        } else {
            parse_ids(v, "memberIds")?
        };
        let mut available = vec![];
        for m in &self.lib.mods {
            if m.game_id == source.game_id && m.folder_id == source.folder_id && !m.retired {
                available.extend(m.members.iter().cloned());
            }
        }
        if wanted
            .iter()
            .any(|id| !available.iter().any(|f| &f.id == id))
        {
            return Err("成员不存在或不属于当前游戏".into());
        }
        let incoming: Vec<_> = available
            .iter()
            .filter(|f| wanted.contains(&f.id))
            .cloned()
            .collect();
        let detached: Vec<_> = source
            .members
            .iter()
            .filter(|f| !wanted.contains(&f.id))
            .cloned()
            .collect();
        for m in &mut self.lib.mods {
            if m.members.iter().any(|f| wanted.contains(&f.id)) {
                m.auto_group = false;
            }
            if m.id != id {
                m.members.retain(|f| !wanted.contains(&f.id));
            }
        }
        self.lib.mods[index].members = incoming;
        for f in detached {
            self.lib.mods.push(Mod {
                id: uid(),
                game_id: source.game_id.clone(),
                folder_id: source.folder_id.clone(),
                name: if ungroup {
                    source.name.clone()
                } else {
                    String::new()
                },
                description: source.description.clone(),
                image_path: source.image_path.clone(),
                original_image_path: source.original_image_path.clone(),
                managed: source.managed,
                state: "enabled".into(),
                members: vec![f],
                retired: source.retired,
                auto_group: false,
            });
        }
        self.lib.mods.retain(|m| !m.members.is_empty());
        self.reconcile();
        self.persist()
    }
    fn export(&self, p: &Path) -> Result<Value, String> {
        if !p.is_absolute() {
            return Err("备份路径应为绝对路径".into());
        }
        reject_links(p)?;
        if self.lib.games.iter().any(|g| {
            g.folders.iter().any(|f| {
                let scoped = folder_game(g, &f.id).unwrap();
                within(p, &scoped.mod_dir) || within(p, &disabled_root(&scoped))
            })
        }) {
            return Err("请将备份保存在游戏 MOD 目录以外".into());
        }
        let mut images = HashMap::new();
        for g in &self.lib.games {
            if let Some(p) = &g.image_path {
                let data = fs::read(p).map_err(error)?;
                images.insert(g.id.clone(),json!({"extension":p.extension().unwrap_or_default().to_string_lossy(),"data":STANDARD.encode(data)}));
            }
        }
        for m in &self.lib.mods {
            if let Some(p) = &m.image_path {
                let data = fs::read(p).map_err(error)?;
                images.insert(format!("mod:{}", m.id),json!({"extension":p.extension().unwrap_or_default().to_string_lossy(),"data":STANDARD.encode(data)}));
            }
        }
        for m in &self.lib.mods {
            if let Some(p) = &m.original_image_path {
                reject_links(p)?;
                let data = fs::read(p).map_err(error)?;
                images.insert(format!("original:{}", m.id),json!({"extension":p.extension().unwrap_or_default().to_string_lossy(),"data":STANDARD.encode(data)}));
            }
        }
        let out = json!({"format":"mod-manager","version":3,"library":self.lib,"images":images});
        let bytes = serde_json::to_vec_pretty(&out).map_err(error)?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("备份超过 64 MB，尚未创建文件。请缩小游戏或 MOD 封面图片后重试".into());
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        use std::io::Write;
        let mut file = options
            .open(p)
            .map_err(|e| format!("备份未写入，请使用不存在的新文件名：{e}"))?;
        file.write_all(&bytes).map_err(error)?;
        file.sync_all().map_err(error)?;
        Ok(json!({"path":p,"success":true}))
    }
    fn import(&mut self, p: &Path) -> Result<(), String> {
        self.ensure_no_journals()?;
        self.reconcile();
        reject_links(p)?;
        let meta = fs::metadata(p).map_err(error)?;
        if meta.len() > 64 * 1024 * 1024 {
            return Err("备份文件过大".into());
        }
        let value: Value = serde_json::from_slice(&fs::read(p).map_err(error)?)
            .map_err(|_| "备份不是有效 JSON 文件")?;
        if value["format"] != "mod-manager"
            || ![json!(1), json!(2), json!(3)].contains(&value["version"])
        {
            return Err("不支持的备份格式或版本".into());
        }
        let mut incoming: Library =
            serde_json::from_value(value["library"].clone()).map_err(|_| "备份结构无效")?;
        normalize_library(&mut incoming);
        validate_library(&incoming)?;
        for g in &incoming.games {
            if self.lib.mods.iter().any(|m| {
                m.game_id == g.id
                    && !m.retired
                    && ((m.state != "enabled" && m.state != "missing")
                        || m.members.iter().any(|f| {
                            f.current_path.exists() && !same(&f.original_path, &f.current_path)
                        }))
            }) {
                return Err(
                    "导入涉及仍禁用或异常的 MOD，请先开启并恢复全部成员，避免覆盖恢复路径".into(),
                );
            }
        }
        // A folder ID must never silently acquire another restore path.
        for g in &incoming.games {
            for folder in &g.folders {
                for existing in &self.lib.games {
                    for ef in &existing.folders {
                        if folder.id == ef.id
                            && (g.id != existing.id || !same(&folder.path, &ef.path))
                        {
                            return Err("备份目录标识指向不同游戏或路径，无法安全合并".into());
                        }
                        if g.id != existing.id
                            && (within(&folder.path, &ef.path) || within(&ef.path, &folder.path))
                        {
                            return Err("导入目录与现有游戏配置重叠".into());
                        }
                    }
                }
            }
        }
        let images = value
            .get("images")
            .and_then(Value::as_object)
            .ok_or("备份缺少图片表")?;
        let mut decoded = vec![];
        for g in &incoming.games {
            if let Some(v) = images.get(&g.id) {
                let ext = string(v, "extension")?.to_lowercase();
                let data = STANDARD
                    .decode(string(v, "data")?)
                    .map_err(|_| "图片编码无效")?;
                if data.len() > 20 * 1024 * 1024 {
                    return Err("备份图片过大".into());
                }
                validate_image(&data, &ext)?;
                decoded.push((g.id.clone(), ext, data));
            } else if g.image_path.is_some() {
                return Err("备份缺少游戏图片数据".into());
            }
        }
        for m in &incoming.mods {
            if let Some(v) = images.get(&format!("mod:{}", m.id)) {
                let ext = string(v, "extension")?.to_lowercase();
                let data = STANDARD
                    .decode(string(v, "data")?)
                    .map_err(|_| "图片编码无效")?;
                if data.len() > 20 * 1024 * 1024 {
                    return Err("备份图片过大".into());
                }
                validate_image(&data, &ext)?;
                decoded.push((format!("mod:{}", m.id), ext, data));
            } else if m.image_path.is_some() {
                return Err("备份缺少 MOD 图片数据".into());
            }
        }
        for m in &incoming.mods {
            if let Some(v) = images.get(&format!("original:{}", m.id)) {
                if m.image_path.is_none() {
                    return Err("原图缺少关联封面".into());
                }
                let ext = string(v, "extension")?.to_lowercase();
                let data = STANDARD
                    .decode(string(v, "data")?)
                    .map_err(|_| "原图编码无效")?;
                if data.len() > 20 * 1024 * 1024 {
                    return Err("备份原图过大".into());
                }
                validate_image(&data, &ext)?;
                decoded.push((format!("original:{}", m.id), ext, data));
            } else if m.original_image_path.is_some() {
                return Err("备份缺少 MOD 原图数据".into());
            }
        }
        let dir = self.data_dir.join("images");
        fs::create_dir_all(&dir).map_err(error)?;
        reject_links(&dir)?;
        for (id, ext, data) in decoded {
            let dest = dir.join(format!("{}-{}.{}", id.replace(':', "-"), uid(), ext));
            fs::write(&dest, data).map_err(error)?;
            if let Some(mod_id) = id.strip_prefix("original:") {
                incoming
                    .mods
                    .iter_mut()
                    .find(|m| m.id == mod_id)
                    .unwrap()
                    .original_image_path = Some(dest);
            } else if let Some(mod_id) = id.strip_prefix("mod:") {
                incoming
                    .mods
                    .iter_mut()
                    .find(|m| m.id == mod_id)
                    .unwrap()
                    .image_path = Some(dest);
            } else {
                incoming
                    .games
                    .iter_mut()
                    .find(|g| g.id == id)
                    .unwrap()
                    .image_path = Some(dest);
            }
        }
        let gids: HashSet<_> = incoming.games.iter().map(|g| g.id.clone()).collect();
        let mut merged = self.lib.clone();
        for g in incoming.games {
            merged.games.retain(|old| old.id != g.id);
            merged.games.push(g);
        }
        merged.mods.retain(|m| !gids.contains(&m.game_id));
        merged.mods.extend(incoming.mods);
        merged.settings = incoming.settings;
        validate_library(&merged)?;
        let raw = serde_json::to_string(&merged).map_err(error)?;
        let tx = self.conn.transaction().map_err(error)?;
        tx.execute("INSERT INTO library(id,data) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET data=excluded.data",params![raw]).map_err(error)?;
        tx.commit().map_err(error)?;
        self.lib = merged;
        self.reconcile();
        self.persist()
    }
    pub fn request(&mut self, v: Value) -> Result<Value, String> {
        self.request_with_progress(v, |_| {})
    }
    pub fn request_with_progress(
        &mut self,
        v: Value,
        progress: impl FnMut(Value),
    ) -> Result<Value, String> {
        let before = self.lib.clone();
        let prior_warnings = self.warnings.clone();
        let ordinary = !matches!(
            v.get("action").and_then(Value::as_str),
            Some("toggle" | "scan")
        );
        let result = if v["action"] == "toggle" {
            self.toggle_batch(&v, progress)
        } else {
            self.dispatch(v)
        };
        if result.is_err() {
            if ordinary {
                self.warnings = prior_warnings;
            }
            self.lib = self
                .conn
                .query_row("SELECT data FROM library WHERE id=1", [], |r| {
                    r.get::<_, String>(0)
                })
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(before);
            self.reconcile();
        }
        result
    }
    fn toggle_batch(
        &mut self,
        v: &Value,
        mut progress: impl FnMut(Value),
    ) -> Result<Value, String> {
        let ids = parse_ids(v, "ids")?;
        let enabled = v["enabled"].as_bool().ok_or("缺少目标状态")?;
        let mut results = vec![];
        let mut mods = vec![];
        for (index, id) in ids.iter().enumerate() {
            let result = self.toggle_one(id, enabled);
            let detail = result.as_ref().err().cloned();
            if result.is_err() {
                self.reconcile_one(id);
            }
            self.conn
                .execute(
                    "INSERT INTO history(mod_id,enabled,result) VALUES(?1,?2,?3)",
                    params![
                        id,
                        enabled,
                        detail.clone().unwrap_or_else(|| "success".into())
                    ],
                )
                .map_err(error)?;
            let result = json!({"id":id,"success":result.is_ok(),"error":detail});
            let updated = self
                .lib
                .mods
                .iter()
                .find(|m| m.id == *id)
                .map(Self::mod_value);
            progress(
                json!({"operationId":v["operationId"],"done":index+1,"total":ids.len(),"result":result,"mod":updated}),
            );
            results.push(result);
            if let Some(updated) = updated {
                mods.push(updated);
            }
        }
        Ok(json!({"results":results,"mods":mods,"warnings":self.warnings}))
    }
    fn dispatch(&mut self, v: Value) -> Result<Value, String> {
        let action = string(&v, "action")?;
        match action.as_str() {
            "snapshot" => return Ok(self.snapshot()),
            "resolveImport" => self.resolve_import(&v)?,
            "maintenance" => self.maintain_storage()?,
            "saveCrop" => return self.save_crop(&v),
            "previewImage" => {
                let p = PathBuf::from(string(&v, "path")?);
                if !p.is_absolute() {
                    return Err("图片路径应为绝对路径".into());
                }
                reject_links(&p)?;
                let meta = fs::metadata(&p).map_err(error)?;
                if !meta.is_file() || meta.len() > 20 * 1024 * 1024 {
                    return Err("请选择不超过 20 MB 的图片文件".into());
                }
                let ext = p
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                let bytes = fs::read(&p).map_err(error)?;
                if bytes.len() > 20 * 1024 * 1024 {
                    return Err("图片不能超过 20 MB".into());
                }
                validate_image(&bytes, &ext)?;
                let mime = if ext == "jpg" || ext == "jpeg" {
                    "jpeg"
                } else {
                    &ext
                };
                return Ok(
                    json!({"url":format!("data:image/{mime};base64,{}", STANDARD.encode(bytes))}),
                );
            }
            "saveGame" => self.save_game(v.get("game").ok_or("缺少游戏配置")?)?,
            "deleteGame" => {
                let id = string(&v, "id")?;
                self.reconcile();
                if self.lib.mods.iter().any(|m| {
                    m.game_id == id
                        && !m.retired
                        && ((m.state != "enabled" && m.state != "missing")
                            || m.members.iter().any(|f| {
                                f.current_path.exists() && !same(&f.current_path, &f.original_path)
                            }))
                }) {
                    return Err("请先开启全部 MOD 再删除游戏配置，禁用成员需要保留恢复路径".into());
                }
                self.ensure_no_journals()?;
                self.lib.games.retain(|g| g.id != id);
                self.lib.mods.retain(|m| m.game_id != id);
                self.persist()?;
            }
            "scan" => self.scan(v.get("gameId").and_then(Value::as_str))?,
            "editMod" => self.edit_mod(&v)?,
            "renameMember" => self.rename_member(&v)?,
            "deleteMod" => self.delete_mod(&v)?,
            "restoreMod" => self.restore_mod(&v)?,
            "importFiles" => return self.import_files(&v),
            "saveView" => {
                let view = v
                    .get("view")
                    .filter(|v| v.is_object())
                    .ok_or("界面位置记录格式错误")?;
                let raw = serde_json::to_string(view).map_err(error)?;
                if raw.len() > 16384 {
                    return Err("界面位置记录过大".into());
                }
                self.conn.execute("INSERT INTO preferences(id,data) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET data=excluded.data",params![raw]).map_err(error)?;
                return Ok(json!({"success":true}));
            }
            "forgetMod" => self.forget_mod(&v)?,
            "acceptReplacement" => self.accept_replacement(&v)?,
            "group" => self.grouping(&v)?,
            "setMembers" => self.members(&v, false)?,
            "ungroup" => self.members(&v, true)?,
            "settings" => {
                if let Some(t) = v.get("theme").and_then(Value::as_str) {
                    if !["system", "light", "dark"].contains(&t) {
                        return Err("未知外观模式".into());
                    }
                    self.lib.settings.theme = t.into();
                }
                if let Some(t) = v.get("defaultStrategy").and_then(Value::as_str) {
                    if !["suffix", "move"].contains(&t) {
                        return Err("未知禁用策略".into());
                    }
                    self.lib.settings.default_strategy = t.into();
                }
                self.persist()?;
            }
            "toggle" => {
                return self.toggle_batch(&v, |_| {});
            }
            "export" => return self.export(&PathBuf::from(string(&v, "path")?)),
            "import" => self.import(&PathBuf::from(string(&v, "path")?))?,
            _ => return Err("未知操作".into()),
        }
        Ok(self.snapshot())
    }
}
fn parse_ids(v: &Value, key: &str) -> Result<Vec<String>, String> {
    let a = v.get(key).and_then(Value::as_array).ok_or("缺少选择列表")?;
    let ids = a
        .iter()
        .map(|x| {
            x.as_str()
                .map(str::to_string)
                .ok_or("选择列表格式无效".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err("选择列表有重复项目".into());
    }
    Ok(ids)
}
fn validate_image(data: &[u8], ext: &str) -> Result<(), String> {
    let valid = match ext {
        "png" => data.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" | "jpeg" => data.starts_with(&[255, 216, 255]),
        "webp" => data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP"),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err("图片格式与文件内容不一致".into())
    }
}
fn validate_roots(games: &[Game]) -> Result<(), String> {
    let mut roots = Vec::new();
    let mut ids = HashSet::new();
    for g in games {
        if g.folders.is_empty() || g.folders.len() > 32 {
            return Err("游戏需包含 1 至 32 个目录".into());
        }
        for f in &g.folders {
            if uuid::Uuid::parse_str(&f.id).is_err()
                || !ids.insert(f.id.clone())
                || f.name.trim().is_empty()
                || f.name.chars().count() > 80
                || !f.path.is_absolute()
                || f.path.parent().is_none()
                || f.path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                || f.path.components().any(|c| {
                    c.as_os_str()
                        .to_string_lossy()
                        .to_lowercase()
                        .starts_with(".mod-manager-disabled-")
                })
            {
                return Err("目录标识、备注名或路径无效".into());
            }
            if games
                .iter()
                .any(|other| other.id == f.id && other.id != g.id)
            {
                return Err("目录标识与其他游戏冲突".into());
            }
            reject_links(&f.path)?;
            let scoped = folder_game(g, &f.id)?;
            reject_links(&disabled_root(&scoped))?;
            roots.push((f.path.clone(), disabled_root(&scoped)));
        }
    }
    for (i, (root, off)) in roots.iter().enumerate() {
        for (j, (other, other_off)) in roots.iter().enumerate() {
            if i != j
                && (within(root, other)
                    || within(other, root)
                    || within(root, other_off)
                    || within(other_off, root)
                    || within(off, other_off)
                    || within(other_off, off))
            {
                return Err("MOD 目录相互重叠，请选择互相独立的文件夹".into());
            }
        }
    }
    Ok(())
}
fn validate_library(lib: &Library) -> Result<(), String> {
    validate_library_for_edit(lib, None)
}
fn validate_library_for_edit(lib: &Library, disk_mod: Option<&str>) -> Result<(), String> {
    validate_roots(&lib.games)?;
    let mut ids = HashSet::new();
    let mut files = HashSet::new();
    for g in &lib.games {
        if uuid::Uuid::parse_str(&g.id).is_err()
            || !ids.insert(g.id.clone())
            || g.name.trim().is_empty()
            || !g.mod_dir.is_absolute()
            || g.mod_dir.parent().is_none()
            || g.mod_dir
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            || !["suffix", "move"].contains(&g.strategy.as_str())
        {
            return Err("备份游戏配置无效".into());
        }
        reject_links(&g.mod_dir)?;
        if let Some(exe) = &g.exe_path {
            if !exe.is_absolute()
                || !exe
                    .extension()
                    .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("exe"))
                || exe
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err("备份游戏 EXE 路径必须为绝对 .exe 路径".into());
            }
            reject_links(exe)?;
        }
        if !g.folders.first().is_some_and(|f| same(&g.mod_dir, &f.path)) {
            return Err("主目录配置与目录列表不一致".into());
        }
        for f in &g.folders {
            if f.id != g.id && !ids.insert(f.id.clone()) {
                return Err("目录标识重复".into());
            }
        }
    }
    for m in &lib.mods {
        let check_disk = disk_mod.is_none_or(|id| id == m.id);
        if uuid::Uuid::parse_str(&m.id).is_err()
            || !ids.insert(m.id.clone())
            || m.members.is_empty()
        {
            return Err("备份 MOD 标识或成员无效".into());
        }
        let g = lib
            .games
            .iter()
            .find(|g| g.id == m.game_id)
            .ok_or("备份 MOD 引用了不存在的游戏")?;
        let scoped;
        let g = if m.retired {
            g
        } else {
            scoped = folder_game(g, &m.folder_id)?;
            &scoped
        };
        for f in &m.members {
            if uuid::Uuid::parse_str(&f.id).is_err()
                || !ids.insert(f.id.clone())
                || !["file", "directory"].contains(&f.kind.as_str())
                || !f.original_path.is_absolute()
                || (!m.retired
                    && (!within(&f.original_path, &g.mod_dir)
                        || same(&f.original_path, &g.mod_dir)))
                || f.original_path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                || (!m.retired && !files.insert(norm(&f.original_path)))
                || !f.current_path.is_absolute()
                || f.current_path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err("备份存在重复成员或越界路径".into());
            }
            if m.retired {
                if m.managed {
                    return Err("旧目录记录不能处于可管理状态".into());
                }
                if check_disk {
                    reject_links(&f.original_path)?;
                    reject_links(&f.current_path)?;
                }
                continue;
            }
            let off = if g.strategy == "suffix" && f.kind == "directory" {
                f.original_path.clone()
            } else if g.strategy == "suffix" && f.kind == "file" {
                suffix_disabled(&f.original_path)
            } else {
                disabled_root(g).join(f.original_path.strip_prefix(&g.mod_dir).map_err(error)?)
            };
            if !same(&f.current_path, &f.original_path) && !same(&f.current_path, &off) {
                return Err("备份成员当前路径不符合禁用策略".into());
            }
            if check_disk {
                reject_links(&f.original_path)?;
                reject_links(&f.current_path)?;
            }
            let mut leaves = HashSet::new();
            for leaf in &f.leaves {
                if f.kind != "directory"
                    || !leaf.original.is_absolute()
                    || !within(&leaf.original, &f.original_path)
                    || same(&leaf.original, &f.original_path)
                    || leaf
                        .original
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                    || (!same(&leaf.current, &leaf.original)
                        && !same(&leaf.current, &suffix_disabled(&leaf.original)))
                    || !leaves.insert(norm(&leaf.original))
                {
                    return Err("文件夹成员清单越界或重复".into());
                }
                if check_disk {
                    reject_links(&leaf.original)?;
                    reject_links(&leaf.current)?;
                }
            }
        }
    }
    if !["system", "light", "dark"].contains(&lib.settings.theme.as_str())
        || !["suffix", "move"].contains(&lib.settings.default_strategy.as_str())
    {
        return Err("备份设置无效".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests;
