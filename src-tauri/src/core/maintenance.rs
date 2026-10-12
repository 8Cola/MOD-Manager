use super::*;
use std::time::{Duration, SystemTime};

const UNRESOLVED: &str = "status NOT IN ('done','failed','resolved','acknowledged')";

pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    let columns = conn
        .prepare("PRAGMA table_info(file_imports)")
        .map_err(error)?
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    for name in ["inventory", "stage_identity"] {
        if !columns.iter().any(|c| c == name) {
            conn.execute(
                &format!("ALTER TABLE file_imports ADD COLUMN {name} TEXT"),
                [],
            )
            .map_err(error)?;
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedCopy {
    path: PathBuf,
    identity: String,
    directory: bool,
}

fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

impl Service {
    pub(super) fn pending_imports(&self) -> Vec<Value> {
        let Ok(mut q) = self.conn.prepare(&format!("SELECT id,source,target,stage,status,detail,inventory,stage_identity FROM file_imports WHERE {UNRESOLVED} ORDER BY at,id")) else { return vec![]; };
        let Ok(rows) = q.query_map([], |r| {
            let stage: String = r.get(3)?;
            let target: String = r.get(2)?;
            let inventory: Option<String> = r.get(6)?;
            let identity: Option<String> = r.get(7)?;
            Ok(json!({"id":r.get::<_,String>(0)?,"source":r.get::<_,String>(1)?,"target":target,"stage":stage,"status":r.get::<_,String>(4)?,"detail":r.get::<_,Option<String>>(5)?,"targetExists":exists(Path::new(&target)),"stageExists":exists(Path::new(&stage)),"cleanupAvailable":inventory.as_deref().is_some_and(|s| serde_json::from_str::<Vec<OwnedCopy>>(s).is_ok()) && identity.is_some()}))
        }) else { return vec![]; };
        rows.filter_map(Result::ok).collect()
    }

    pub(super) fn resolve_import(&mut self, v: &Value) -> Result<(), String> {
        if v["confirmed"].as_bool() != Some(true) {
            return Err("请明确确认处理此次中断导入".into());
        }
        let cleanup = v["cleanup"]
            .as_bool()
            .ok_or("请选择保留暂存副本或安全清理")?;
        let id = string(v, "id")?;
        let (target,stage,manifest,identity,source): (String,String,Option<String>,Option<String>,String) = self.conn.query_row(
            &format!("SELECT target,stage,inventory,stage_identity,source FROM file_imports WHERE id=?1 AND {UNRESOLVED}"), params![id],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_| "中断导入记录不存在或已处理".to_string())?;
        if cleanup {
            let stage = PathBuf::from(&stage);
            let target = PathBuf::from(&target);
            let uuid = uuid::Uuid::parse_str(&id).map_err(|_| "暂存记录标识无效")?;
            if uuid.to_string() != id
                || !stage.is_absolute()
                || !target.is_absolute()
                || stage
                    .components()
                    .chain(target.components())
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err("暂存记录路径无效，无法自动清理".into());
            }
            let root = target.parent().ok_or("目标目录无效")?;
            let expected = root
                .parent()
                .ok_or("暂存目录无效")?
                .join(format!(".mod-manager-import-{id}"));
            if !same(&stage, &expected)
                || !self
                    .lib
                    .games
                    .iter()
                    .flat_map(|g| &g.folders)
                    .any(|f| same(&f.path, root))
                || self
                    .lib
                    .games
                    .iter()
                    .flat_map(|g| &g.folders)
                    .any(|f| within(&stage, &f.path))
                || within(&stage, Path::new(&source))
                || within(Path::new(&source), &stage)
            {
                return Err("目录配置已变化，无法安全清理；可确认保留副本".into());
            }
            let inventory: Vec<OwnedCopy> = serde_json::from_str(
                manifest
                    .as_deref()
                    .ok_or("旧记录没有可信清单，请确认保留副本")?,
            )
            .map_err(error)?;
            if inventory.len() > 100_000 {
                return Err("暂存清单过大，请确认保留副本".into());
            }
            let identity = identity.ok_or("没有暂存目录身份，请确认保留副本")?;
            reject_links(&stage)?;
            reject_links(root)?;
            // Validate the entire tree before deleting any recorded member.
            let mut known = HashSet::new();
            for entry in &inventory {
                if !entry.path.is_absolute()
                    || entry
                        .path
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                    || !within(&entry.path, &stage.join("payload"))
                    || !known.insert(norm(&entry.path))
                    || entry.identity.is_empty()
                {
                    return Err("暂存清单包含未知路径，已保留全部副本".into());
                }
                reject_links(&entry.path)?;
                if exists(&entry.path) {
                    let m = fs::symlink_metadata(&entry.path).map_err(error)?;
                    if disk_identity(&entry.path).as_deref() != Some(&entry.identity)
                        || (entry.directory && !m.is_dir())
                        || (!entry.directory && !m.is_file())
                    {
                        return Err("暂存成员身份或类型改变，已保留全部副本".into());
                    }
                }
            }
            if exists(&stage) {
                if !stage.is_dir() || disk_identity(&stage).as_deref() != Some(&identity) {
                    return Err("暂存目录身份改变，已保留全部副本".into());
                }
                ensure_known_tree(&stage, &stage, &known, 0)?;
                let mut ordered: Vec<_> = inventory.iter().collect();
                ordered.sort_by_key(|e| std::cmp::Reverse(e.path.components().count()));
                for entry in ordered {
                    if !exists(&entry.path) {
                        continue;
                    }
                    reject_links(&entry.path)?;
                    if disk_identity(&entry.path).as_deref() != Some(&entry.identity) {
                        return Err("暂存成员在清理期间改变，已停止".into());
                    }
                    if entry.directory {
                        fs::remove_dir(&entry.path)
                    } else {
                        fs::remove_file(&entry.path)
                    }
                    .map_err(error)?;
                }
                reject_links(&stage)?;
                if disk_identity(&stage).as_deref() != Some(&identity) {
                    return Err("暂存目录在清理期间改变，已停止".into());
                }
                fs::remove_dir(&stage).map_err(error)?;
            }
        }
        self.conn
            .execute(
                "UPDATE file_imports SET status=?2,detail=?3 WHERE id=?1",
                params![
                    id,
                    if cleanup { "resolved" } else { "acknowledged" },
                    if cleanup {
                        "用户确认清理管理器暂存副本"
                    } else {
                        "用户确认保留暂存副本"
                    }
                ],
            )
            .map_err(error)?;
        self.warnings.retain(|w| !w.contains(&stage));
        Ok(())
    }

    pub(super) fn save_crop(&self, v: &Value) -> Result<Value, String> {
        let encoded = v["dataUrl"].as_str().ok_or("缺少裁剪图片")?;
        if encoded.len() > 3 * 1024 * 1024 {
            return Err("裁剪图片过大，请降低分辨率".into());
        }
        let data = STANDARD
            .decode(
                encoded
                    .strip_prefix("data:image/png;base64,")
                    .ok_or("裁剪图片必须为 PNG")?,
            )
            .map_err(error)?;
        validate_crop_png(&data)?;
        // Retain the exact uncropped bytes supplied by the picker or clipboard.
        let original = match v.get("sourceUrl") {
            Some(Value::String(url)) => {
                if url.len() > 28 * 1024 * 1024 {
                    return Err("原图不能超过 20 MB".into());
                }
                let (prefix, encoded) = url.split_once(",").ok_or("原图格式无效")?;
                let ext = match prefix {
                    "data:image/png;base64" => "png",
                    "data:image/jpeg;base64" | "data:image/jpg;base64" => "jpg",
                    "data:image/webp;base64" => "webp",
                    _ => return Err("原图格式不支持".into()),
                };
                let bytes = STANDARD.decode(encoded).map_err(|_| "原图编码无效")?;
                if bytes.len() > 20 * 1024 * 1024 {
                    return Err("原图不能超过 20 MB".into());
                }
                validate_image(&bytes, ext)?;
                Some((ext, bytes))
            }
            Some(_) => return Err("原图格式无效".into()),
            None => None,
        };
        let dir = self.data_dir.join("images");
        reject_links(&dir)?;
        fs::create_dir_all(&dir).map_err(error)?;
        let original_path = if let Some((ext, bytes)) = original {
            let path = dir.join(format!("original-{}.{}", uid(), ext));
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(error)?;
            file.write_all(&bytes).map_err(error)?;
            file.sync_all().map_err(error)?;
            Some(path)
        } else {
            None
        };
        let path = dir.join(format!("crop-{}.png", uid()));
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(error)?;
        file.write_all(&data).map_err(error)?;
        file.sync_all().map_err(error)?;
        Ok(json!({"path":path,"originalPath":original_path}))
    }

    pub(super) fn maintain_storage(&mut self) -> Result<(), String> {
        if self
            .last_maintenance
            .is_some_and(|t| t.elapsed() < Duration::from_secs(3600))
        {
            return Ok(());
        }
        self.last_maintenance = Some(std::time::Instant::now());
        self.conn.execute("DELETE FROM history WHERE id NOT IN (SELECT id FROM history ORDER BY id DESC LIMIT 1000)",[]).map_err(error)?;
        self.conn.execute("DELETE FROM file_imports WHERE status IN ('done','failed','resolved','acknowledged') AND id NOT IN (SELECT id FROM file_imports ORDER BY at DESC,id DESC LIMIT 300)",[]).map_err(error)?;
        self.collect_images(SystemTime::now())
    }

    fn collect_images(&self, now: SystemTime) -> Result<(), String> {
        let dir = self.data_dir.join("images");
        if !dir.exists() {
            return Ok(());
        }
        reject_links(&dir)?;
        let mut protected = HashSet::new();
        collect_strings(
            &serde_json::to_value(&self.lib).map_err(error)?,
            &mut protected,
        );
        for (table, column) in [
            ("deletions", "data"),
            ("journals", "plan"),
            ("file_edits", "data"),
        ] {
            let mut q = self
                .conn
                .prepare(&format!("SELECT {column} FROM {table}"))
                .map_err(error)?;
            let rows = q.query_map([], |r| r.get::<_, String>(0)).map_err(error)?;
            for raw in rows {
                let raw = raw.map_err(error)?;
                let Ok(value) = serde_json::from_str::<Value>(&raw) else {
                    return Ok(());
                };
                let valid =
                    match table {
                        "deletions" => serde_json::from_value::<Mod>(value.clone()).is_ok(),
                        "journals" => serde_json::from_value::<Vec<Step>>(value.clone()).is_ok(),
                        "file_edits" => {
                            value.get("before").is_some_and(|v| {
                                serde_json::from_value::<Library>(v.clone()).is_ok()
                            }) && value.get("steps").is_some_and(|v| {
                                serde_json::from_value::<Vec<Step>>(v.clone()).is_ok()
                            }) && value.get("identities").is_some_and(|v| {
                                serde_json::from_value::<Vec<String>>(v.clone()).is_ok()
                            })
                        }
                        _ => false,
                    };
                if !valid {
                    return Ok(());
                }
                collect_strings(&value, &mut protected);
            }
        }
        // Unknown imports may contain paths in undocumented recovery metadata.
        let unresolved: i64 = self
            .conn
            .query_row(
                &format!("SELECT COUNT(*) FROM file_imports WHERE {UNRESOLVED}"),
                [],
                |r| r.get(0),
            )
            .map_err(error)?;
        if unresolved != 0 {
            return Ok(());
        }
        let mut inspected = 0usize;
        for entry in fs::read_dir(&dir).map_err(error)? {
            if inspected >= 1000 {
                break;
            }
            inspected += 1;
            let path = entry.map_err(error)?.path();
            if protected.contains(&norm(&path))
                || !managed_image_name(&path)
                || reject_links(&path).is_err()
            {
                continue;
            }
            let meta = fs::symlink_metadata(&path).map_err(error)?;
            if !meta.is_file()
                || now
                    .duration_since(meta.modified().map_err(error)?)
                    .unwrap_or_default()
                    < Duration::from_secs(86400)
            {
                continue;
            }
            let Some(identity) = disk_identity(&path) else {
                continue;
            };
            reject_links(&path)?;
            if disk_identity(&path).as_deref() == Some(&identity) {
                fs::remove_file(&path).map_err(error)?;
            }
        }
        Ok(())
    }
}

// This bounds the PNG container and its crop dimensions; it does not decode pixels.
fn validate_crop_png(data: &[u8]) -> Result<(), String> {
    let invalid = || "裁剪图片应为完整的 512 × 512 PNG".to_string();
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(invalid());
    }
    let mut offset = 8usize;
    let mut chunks = 0usize;
    let mut color = 0u8;
    let mut depth = 0u8;
    let mut palette = false;
    let mut image_data = 0usize;
    let mut seen_data = false;
    let mut data_closed = false;
    let mut zlib_header = Vec::with_capacity(2);
    while offset < data.len() {
        chunks += 1;
        if chunks > 4096 {
            return Err(invalid());
        }
        let header_end = offset.checked_add(8).ok_or_else(invalid)?;
        let header = data.get(offset..header_end).ok_or_else(invalid)?;
        let length = u32::from_be_bytes(header[..4].try_into().map_err(|_| invalid())?) as usize;
        let kind = &header[4..8];
        let content_end = header_end.checked_add(length).ok_or_else(invalid)?;
        let next = content_end.checked_add(4).ok_or_else(invalid)?;
        let content = data.get(header_end..content_end).ok_or_else(invalid)?;
        if next > data.len()
            || !kind.iter().all(u8::is_ascii_alphabetic)
            || !kind[2].is_ascii_uppercase()
        {
            return Err(invalid());
        }
        if chunks == 1 && (kind != b"IHDR" || length != 13) {
            return Err(invalid());
        }
        match kind {
            b"IHDR" => {
                if chunks != 1
                    || length != 13
                    || content[..4] != 512u32.to_be_bytes()
                    || content[4..8] != 512u32.to_be_bytes()
                    || content[10] != 0
                    || content[11] != 0
                    || content[12] > 1
                {
                    return Err(invalid());
                }
                depth = content[8];
                color = content[9];
                let legal_depth = match color {
                    0 => [1, 2, 4, 8, 16].contains(&depth),
                    2 | 4 | 6 => [8, 16].contains(&depth),
                    3 => [1, 2, 4, 8].contains(&depth),
                    _ => false,
                };
                if !legal_depth {
                    return Err(invalid());
                }
            }
            b"PLTE" => {
                if palette
                    || seen_data
                    || ![2, 3, 6].contains(&color)
                    || length == 0
                    || length % 3 != 0
                    || length > 768
                    || (color == 3 && length / 3 > (1usize << depth))
                {
                    return Err(invalid());
                }
                palette = true;
            }
            b"IDAT" => {
                if data_closed || (color == 3 && !palette) {
                    return Err(invalid());
                }
                image_data = image_data.checked_add(length).ok_or_else(invalid)?;
                seen_data = true;
                for byte in content.iter().take(2 - zlib_header.len()) {
                    zlib_header.push(*byte);
                }
            }
            b"IEND" => {
                if length != 0 || next != data.len() || image_data < 6 || zlib_header.len() != 2 {
                    return Err(invalid());
                }
                let cmf = zlib_header[0];
                let flg = zlib_header[1];
                if cmf & 15 != 8
                    || cmf >> 4 > 7
                    || flg & 32 != 0
                    || u16::from_be_bytes([cmf, flg]) % 31 != 0
                {
                    return Err(invalid());
                }
                return Ok(());
            }
            _ => {
                if kind[0].is_ascii_uppercase() {
                    return Err(invalid());
                }
                if seen_data {
                    data_closed = true;
                }
            }
        }
        offset = next;
    }
    Err(invalid())
}

fn ensure_known_tree(
    root: &Path,
    path: &Path,
    known: &HashSet<String>,
    depth: usize,
) -> Result<(), String> {
    if depth > 128 {
        return Err("暂存目录层级过深，请确认保留副本".into());
    }
    reject_links(path)?;
    if !same(root, path) && !known.contains(&norm(path)) {
        return Err("暂存目录含清单以外的文件，已保留全部副本".into());
    }
    if path.is_dir() {
        for entry in fs::read_dir(path).map_err(error)? {
            ensure_known_tree(root, &entry.map_err(error)?.path(), known, depth + 1)?;
        }
    }
    Ok(())
}
fn collect_strings(value: &Value, paths: &mut HashSet<String>) {
    match value {
        Value::String(s) => {
            paths.insert(norm(Path::new(s)));
        }
        Value::Array(a) => {
            for v in a {
                collect_strings(v, paths);
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                collect_strings(v, paths);
            }
        }
        _ => {}
    }
}
fn managed_image_name(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or_default();
    if !["png", "jpg", "jpeg", "webp"].contains(&ext) {
        return false;
    }
    let stem = path
        .file_stem()
        .and_then(|x| x.to_str())
        .unwrap_or_default();
    let Some(start) = stem.len().checked_sub(36) else {
        return false;
    };
    let Some(suffix) = stem.get(start..) else {
        return false;
    };
    start > 0
        && stem.as_bytes().get(start - 1) == Some(&b'-')
        && uuid::Uuid::parse_str(suffix).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Service, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("mods");
        fs::create_dir(&root).unwrap();
        let mut service = Service::new(temp.path().join("app")).unwrap();
        service.lib.games.push(Game {
            id: uid(),
            name: "Test".into(),
            mod_dir: root.clone(),
            image_path: None,
            exe_path: None,
            strategy: "suffix".into(),
            folders: vec![ModFolder {
                id: uid(),
                name: "Mods".into(),
                path: root.clone(),
            }],
        });
        (temp, service, root)
    }
    fn pending(
        service: &Service,
        root: &Path,
        manifest: bool,
    ) -> (String, PathBuf, PathBuf, PathBuf) {
        let id = uid();
        let stage = root
            .parent()
            .unwrap()
            .join(format!(".mod-manager-import-{id}"));
        let source = root.parent().unwrap().join("source.pak");
        let target = root.join("source.pak");
        fs::write(&source, b"source").unwrap();
        fs::write(&target, b"published").unwrap();
        fs::create_dir(&stage).unwrap();
        let payload = stage.join("payload");
        fs::write(&payload, b"temporary").unwrap();
        let inventory = manifest.then(|| json!([{"path":payload,"identity":disk_identity(&payload).unwrap(),"directory":false}]).to_string());
        service.conn.execute("INSERT INTO file_imports(id,source,target,stage,status,inventory,stage_identity) VALUES(?1,?2,?3,?4,'pending',?5,?6)",params![id,source.to_string_lossy(),target.to_string_lossy(),stage.to_string_lossy(),inventory,disk_identity(&stage)]).unwrap();
        (id, stage, source, target)
    }
    #[test]
    fn acknowledge_legacy_ends_warning_without_touching_any_files() {
        let (_temp, mut s, root) = setup();
        let (id, stage, source, target) = pending(&s, &root, false);
        s.warnings
            .push(format!("上次拖放导入中断 {}", stage.display()));
        assert!(s
            .resolve_import(&json!({"id":id,"confirmed":true,"cleanup":true}))
            .is_err());
        assert!(stage.exists());
        s.resolve_import(&json!({"id":id,"confirmed":true,"cleanup":false}))
            .unwrap();
        assert!(s.pending_imports().is_empty());
        assert!(s.warnings.is_empty());
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(target).unwrap(), b"published");
        assert!(stage.join("payload").exists());
    }
    #[test]
    fn cleanup_only_removes_recorded_copies() {
        let (_temp, mut s, root) = setup();
        let (id, stage, source, target) = pending(&s, &root, true);
        s.resolve_import(&json!({"id":id,"confirmed":true,"cleanup":true}))
            .unwrap();
        assert!(!stage.exists());
        assert!(s.pending_imports().is_empty());
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(target).unwrap(), b"published");
    }
    #[test]
    fn unknown_file_changed_identity_and_moved_configuration_preserve_copies() {
        for reason in [
            "unknown",
            "changed",
            "configuration",
            "stage",
            "stagepath",
            "manifest",
        ] {
            let (_temp, mut s, root) = setup();
            let (id, stage, _, _) = pending(&s, &root, true);
            match reason {
                "unknown" => fs::write(stage.join("outside.txt"), b"unknown").unwrap(),
                "changed" => {
                    fs::rename(stage.join("payload"), stage.join("original")).unwrap();
                    fs::write(stage.join("payload"), b"replacement").unwrap();
                    fs::remove_file(stage.join("original")).unwrap();
                }
                "configuration" => s.lib.games.clear(),
                "stage" => {
                    s.conn
                        .execute(
                            "UPDATE file_imports SET stage_identity='unknown' WHERE id=?1",
                            params![id],
                        )
                        .unwrap();
                }
                "stagepath" => {
                    s.conn
                        .execute(
                            "UPDATE file_imports SET stage=?2 WHERE id=?1",
                            params![id, root.to_string_lossy()],
                        )
                        .unwrap();
                }
                "manifest" => {
                    s.conn.execute("UPDATE file_imports SET inventory=?2 WHERE id=?1",params![id,json!([{"path":root.join("source.pak"),"identity":"untrusted","directory":false}]).to_string()]).unwrap();
                }
                _ => unreachable!(),
            }
            assert!(
                s.resolve_import(&json!({"id":id,"confirmed":true,"cleanup":true}))
                    .is_err(),
                "{reason}"
            );
            assert!(stage.join("payload").exists(), "{reason}");
            assert_eq!(s.pending_imports().len(), 1);
        }
    }
    #[test]
    fn linked_stage_is_preserved() {
        let (_temp, mut s, root) = setup();
        let (id, stage, _, _) = pending(&s, &root, true);
        let other = stage.with_extension("other");
        fs::rename(&stage, &other).unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::fs::symlink_dir;
            if symlink_dir(&other, &stage).is_err() {
                let output = std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(&stage)
                    .arg(&other)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "cannot create junction for recovery safety test"
                );
            }
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&other, &stage).unwrap();
        assert!(s
            .resolve_import(&json!({"id":id,"confirmed":true,"cleanup":true}))
            .is_err());
        assert!(other.join("payload").exists());
    }
    fn image(s: &Service) -> PathBuf {
        let dir = s.data_dir.join("images");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("crop-{}.png", uid()));
        fs::write(&path, b"image").unwrap();
        path
    }
    fn historical_mod(image: &Path) -> Mod {
        Mod {
            id: uid(),
            game_id: uid(),
            folder_id: uid(),
            name: "old".into(),
            description: "".into(),
            image_path: Some(image.to_path_buf()),
            original_image_path: None,
            managed: true,
            state: "missing".into(),
            members: vec![],
            retired: true,
            auto_group: false,
        }
    }
    #[test]
    fn image_collection_protects_retired_and_all_recovery_records_and_grace() {
        let (_temp, mut s, _root) = setup();
        let retired = image(&s);
        let audit = image(&s);
        let edit = image(&s);
        let journal = image(&s);
        let orphan = image(&s);
        s.lib.mods.push(historical_mod(&retired));
        s.conn
            .execute(
                "INSERT INTO deletions(id,data,status) VALUES(?1,?2,'done')",
                params![
                    uid(),
                    serde_json::to_string(&historical_mod(&audit)).unwrap()
                ],
            )
            .unwrap();
        let mut before = s.lib.clone();
        before.mods.push(historical_mod(&edit));
        s.conn
            .execute(
                "INSERT INTO file_edits(id,data) VALUES(?1,?2)",
                params![
                    uid(),
                    json!({"before":before,"steps":[],"identities":[]}).to_string()
                ],
            )
            .unwrap();
        s.conn
            .execute(
                "INSERT INTO journals(id,plan) VALUES(?1,?2)",
                params![uid(), json!([{"from":journal,"to":journal}]).to_string()],
            )
            .unwrap();
        s.collect_images(SystemTime::now()).unwrap();
        assert!(orphan.exists());
        s.collect_images(SystemTime::now() + Duration::from_secs(86401))
            .unwrap();
        for p in [retired, audit, edit, journal] {
            assert!(p.exists());
        }
        assert!(!orphan.exists());
    }
    #[test]
    fn invalid_recovery_data_and_unknown_images_block_untrusted_deletion() {
        let (_temp, s, _root) = setup();
        let orphan = image(&s);
        let unknown = s.data_dir.join("images/manual.png");
        fs::write(&unknown, b"manual").unwrap();
        s.conn
            .execute(
                "INSERT INTO file_edits(id,data) VALUES(?1,'{}')",
                params![uid()],
            )
            .unwrap();
        s.collect_images(SystemTime::now() + Duration::from_secs(86401))
            .unwrap();
        assert!(orphan.exists());
        assert!(unknown.exists());
    }
    #[test]
    fn bounded_retention_keeps_every_unresolved_import() {
        let (_temp, mut s, _root) = setup();
        for i in 0..1050 {
            s.conn
                .execute(
                    "INSERT INTO history(id,result) VALUES(?1,'success')",
                    params![i],
                )
                .unwrap();
        }
        for i in 0..350 {
            s.conn.execute("INSERT INTO file_imports(id,source,target,stage,status) VALUES(?1,'source','target','stage','done')",params![i.to_string()]).unwrap();
        }
        for status in ["pending", "copying", "cleanup_required", "unknown"] {
            s.conn.execute("INSERT INTO file_imports(id,source,target,stage,status) VALUES(?1,'source','target','stage',?1)",params![status]).unwrap();
        }
        s.last_maintenance = None;
        s.maintain_storage().unwrap();
        let history: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(history, 1000);
        assert_eq!(s.pending_imports().len(), 4);
        let completed: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM file_imports WHERE status='done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(completed <= 300);
    }
    #[test]
    fn crop_rejects_wrong_format_and_saves_a_new_file() {
        let (_temp, s, _root) = setup();
        assert!(s
            .save_crop(&json!({"dataUrl":"data:image/jpeg;base64,AAAA"}))
            .is_err());
        let bytes = include_bytes!("../../tests/fixtures/crop512.png");
        let result = s
            .save_crop(
                &json!({"dataUrl":format!("data:image/png;base64,{}",STANDARD.encode(bytes))}),
            )
            .unwrap();
        assert!(Path::new(result["path"].as_str().unwrap()).exists());
    }
    #[test]
    fn crop_rejects_truncated_dimensions_and_invalid_chunks_without_writing() {
        let (_temp, s, _root) = setup();
        let valid = include_bytes!("../../tests/fixtures/crop512.png");
        let mut wrong_dimensions = valid.to_vec();
        wrong_dimensions[16..20].copy_from_slice(&513u32.to_be_bytes());
        let mut oversized_chunk = valid.to_vec();
        oversized_chunk[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        let mut unknown_critical_chunk = valid.to_vec();
        unknown_critical_chunk[12..16].copy_from_slice(b"ABCD");
        let mut duplicate_header = valid[..33].to_vec();
        duplicate_header.extend_from_slice(&valid[8..]);
        let mut trailing_data = valid.to_vec();
        trailing_data.push(0);
        let mut missing_data = valid[..33].to_vec();
        missing_data.extend_from_slice(&valid[valid.len() - 12..]);
        let mut invalid_depth = valid.to_vec();
        invalid_depth[24] = 7;
        for bytes in [
            b"\x89PNG\r\n\x1a\n".to_vec(),
            valid[..valid.len() - 1].to_vec(),
            wrong_dimensions,
            oversized_chunk,
            unknown_critical_chunk,
            duplicate_header,
            trailing_data,
            missing_data,
            invalid_depth,
        ] {
            assert!(s
                .save_crop(
                    &json!({"dataUrl":format!("data:image/png;base64,{}",STANDARD.encode(bytes))})
                )
                .is_err());
            assert!(!s.data_dir.join("images").exists());
        }
        assert!(s
            .save_crop(
                &json!({"dataUrl":format!("data:image/png;base64,{}","A".repeat(3*1024*1024))})
            )
            .is_err());
        assert!(!s.data_dir.join("images").exists());
    }
}
