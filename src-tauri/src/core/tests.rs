use super::*;
use serde_json::json;
use tempfile::tempdir;
#[test]
fn batch_progress_is_verified_per_item_and_reports_partial_failure() {
    let (_t, mut s, g, r) = setup();
    for name in ["a.pak", "b.pak"] {
        fs::write(r.join(name), name).unwrap();
    }
    s.request(json!({"action":"scan","gameId":g})).unwrap();
    let a = s
        .lib
        .mods
        .iter()
        .find(|m| same(&m.members[0].original_path, &r.join("a.pak")))
        .unwrap()
        .id
        .clone();
    let b = s
        .lib
        .mods
        .iter()
        .find(|m| same(&m.members[0].original_path, &r.join("b.pak")))
        .unwrap()
        .id
        .clone();
    fs::write(r.join("b.pak.disabled"), "conflict").unwrap();
    let mut events = vec![];
    let out = s
        .request_with_progress(
            json!({"action":"toggle","ids":[a,b],"enabled":false,"operationId":"probe"}),
            |e| {
                if e["result"]["success"] == true {
                    assert!(r.join("a.pak.disabled").exists());
                    assert!(!r.join("a.pak").exists());
                }
                events.push(e);
            },
        )
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["operationId"], "probe");
    assert_eq!(events[0]["done"], 1);
    assert_eq!(events[1]["done"], 2);
    assert_eq!(events[0]["mod"]["state"], "disabled");
    assert_eq!(events[1]["mod"]["state"], "error");
    assert_eq!(out["results"][0]["success"], true);
    assert_eq!(out["results"][1]["success"], false);
    assert_eq!(fs::read(r.join("b.pak.disabled")).unwrap(), b"conflict");
}
#[test]
fn toggle_updates_only_target_and_snapshot_still_detects_unrelated_changes() {
    let (_t, mut s, g, r) = setup();
    fs::write(r.join("target.pak"), "target").unwrap();
    fs::create_dir(r.join("unrelated")).unwrap();
    fs::write(r.join("unrelated/old.pak"), "old").unwrap();
    s.request(json!({"action":"scan","gameId":g})).unwrap();
    let id = s
        .lib
        .mods
        .iter()
        .find(|m| same(&m.members[0].original_path, &r.join("target.pak")))
        .unwrap()
        .id
        .clone();
    let unrelated = s
        .lib
        .mods
        .iter()
        .find(|m| m.members[0].kind == "directory")
        .unwrap()
        .id
        .clone();
    fs::write(r.join("unrelated/new.pak"), "new").unwrap();
    let out = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(out["results"][0]["success"], true);
    assert_eq!(out["mods"][0]["state"], "disabled");
    assert_eq!(out["mods"].as_array().unwrap().len(), 1);
    assert_eq!(
        s.lib
            .mods
            .iter()
            .find(|m| m.id == unrelated)
            .unwrap()
            .members[0]
            .leaves
            .len(),
        1
    );
    s.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(
        s.lib
            .mods
            .iter()
            .find(|m| m.id == unrelated)
            .unwrap()
            .members[0]
            .leaves
            .len(),
        2
    );
}
#[test]
#[ignore = "isolated timing probe; run explicitly with --ignored --nocapture"]
fn toggle_timing_probe() {
    let (t, mut s, g, r) = setup();
    for game in 0..3 {
        let root = if game == 0 {
            r.clone()
        } else {
            t.path().join(format!("other-{game}"))
        };
        fs::create_dir_all(&root).unwrap();
        for bundle in 0..5 {
            let folder = root.join(format!("bundle-{bundle}"));
            fs::create_dir(&folder).unwrap();
            for file in 0..40 {
                fs::write(folder.join(format!("{file}.pak")), "fixture").unwrap();
            }
        }
        if game != 0 {
            s.request(json!({"action":"saveGame","game":{"name":format!("other-{game}"),"modDir":root,"strategy":"suffix"}})).unwrap();
        }
    }
    fs::write(r.join("single.pak"), "single").unwrap();
    let exe = t.path().join("not-running.exe");
    fs::write(&exe, "test").unwrap();
    s.lib.games.iter_mut().find(|x| x.id == g).unwrap().exe_path = Some(exe);
    s.request(json!({"action":"scan"})).unwrap();
    for m in &mut s.lib.mods {
        m.managed = true;
    }
    let single = s
        .lib
        .mods
        .iter()
        .find(|m| same(&m.members[0].original_path, &r.join("single.pak")))
        .unwrap()
        .id
        .clone();
    let folder = s
        .lib
        .mods
        .iter()
        .find(|m| same(&m.members[0].original_path, &r.join("bundle-0")))
        .unwrap()
        .id
        .clone();
    let batch: Vec<_> = s
        .lib
        .mods
        .iter()
        .filter(|m| m.game_id == g)
        .map(|m| m.id.clone())
        .collect();
    for (name, ids) in [
        ("single", vec![single]),
        ("folder40", vec![folder]),
        ("batch6", batch),
    ] {
        let mut times = vec![];
        for enabled in [false, true, false, true] {
            let start = std::time::Instant::now();
            // Old frontend sends one request per item and waits for a full snapshot.
            // The optimized protocol supplies verified patches in the batch response.
            let optimized = std::env::var_os("MM_FAST_PROBE").is_some();
            if optimized {
                let out = s
                    .request(json!({"action":"toggle","ids":ids,"enabled":enabled}))
                    .unwrap();
                assert!(out["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|x| x["success"] == true));
            } else {
                for id in &ids {
                    let out = s
                        .request(json!({"action":"toggle","ids":[id],"enabled":enabled}))
                        .unwrap();
                    assert_eq!(out["results"][0]["success"], true);
                }
                s.request(json!({"action":"snapshot"})).unwrap();
            }
            times.push(start.elapsed().as_millis());
        }
        println!("TIMING {name}: {times:?} ms");
    }
}
#[test]
fn original_cover_survives_restart_and_backup_and_removal_clears_both() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let (t, mut s, g, r) = setup();
    fs::write(r.join("cover.pak"), "mod").unwrap();
    let snap = s.request(json!({"action":"scan","gameId":g})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    let bytes = include_bytes!("../../tests/fixtures/crop512.png");
    let url = format!("data:image/png;base64,{}", STANDARD.encode(bytes));
    let crop = s
        .request(json!({"action":"saveCrop","dataUrl":url,"sourceUrl":url}))
        .unwrap();
    let out = s.request(json!({"action":"editMod","id":id,"imagePath":crop["path"],"originalImagePath":crop["originalPath"]})).unwrap();
    let original = PathBuf::from(out["mods"][0]["originalImagePath"].as_str().unwrap());
    assert_eq!(fs::read(&original).unwrap(), bytes);
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    assert_eq!(s.lib.mods[0].original_image_path.as_ref(), Some(&original));
    let backup = t.path().join("backup.json");
    s.request(json!({"action":"export","path":backup})).unwrap();
    let packed: Value = serde_json::from_slice(&fs::read(&backup).unwrap()).unwrap();
    assert!(packed["images"][format!("original:{}", id.as_str().unwrap())].is_object());
    let out = s.request(json!({"action":"import","path":backup})).unwrap();
    let restored = PathBuf::from(out["mods"][0]["originalImagePath"].as_str().unwrap());
    assert_eq!(fs::read(restored).unwrap(), bytes);
    let out = s
        .request(json!({"action":"editMod","id":id,"imagePath":null}))
        .unwrap();
    assert!(out["mods"][0]["imagePath"].is_null());
    assert!(out["mods"][0]["originalImagePath"].is_null());
}
#[test]
fn invalid_original_is_rejected_before_creating_assets() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let (t, mut s, _, _) = setup();
    let url = format!(
        "data:image/png;base64,{}",
        STANDARD.encode(include_bytes!("../../tests/fixtures/crop512.png"))
    );
    assert!(s
        .request(
            json!({"action":"saveCrop","dataUrl":url,"sourceUrl":"data:image/png;base64,YmFk"})
        )
        .is_err());
    assert!(!t.path().join("data/images").exists());
}
#[test]
fn drop_import_copies_files_archives_and_folder_to_selected_directory() {
    let (t, mut s, g, r) = setup();
    let incoming = t.path().join("downloads");
    fs::create_dir(&incoming).unwrap();
    fs::create_dir(incoming.join("bundle")).unwrap();
    fs::write(incoming.join("bundle/a.txt"), "folder").unwrap();
    fs::write(incoming.join("a.pak"), "pak").unwrap();
    fs::write(incoming.join("pack.zip"), "archive").unwrap();
    let second = t.path().join("shaders");
    fs::create_dir(&second).unwrap();
    let first = s.lib.games[0].folders[0].clone();
    let snap=s.request(json!({"action":"saveGame","game":{"id":g,"name":"Test","strategy":"suffix","folders":[first,{"name":"光影","path":second}]}})).unwrap();
    let folder = snap["games"][0]["folders"][1]["id"].clone();
    let out=s.request(json!({"action":"importFiles","gameId":g,"folderId":folder,"paths":[incoming.join("a.pak"),incoming.join("pack.zip"),incoming.join("bundle")]})).unwrap();
    assert!(out["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|x| x["success"] == true));
    for file in ["a.pak", "pack.zip", "bundle/a.txt"] {
        assert_eq!(
            fs::read(second.join(file)).unwrap(),
            fs::read(incoming.join(file)).unwrap()
        );
        assert!(!r.join(file).exists());
    }
    assert_eq!(s.lib.mods.len(), 3);
}
#[test]
fn drop_import_preflight_never_overwrites_or_partially_imports_conflicts() {
    let (t, mut s, g, r) = setup();
    let source = t.path().join("a.pak");
    fs::write(&source, "new").unwrap();
    fs::write(r.join("a.pak"), "old").unwrap();
    let other = t.path().join("b.pak");
    fs::write(&other, "b").unwrap();
    let f = s.lib.games[0].folders[0].id.clone();
    assert!(s
        .request(json!({"action":"importFiles","gameId":g,"folderId":f,"paths":[other,source]}))
        .unwrap_err()
        .contains("已存在"));
    assert_eq!(fs::read(r.join("a.pak")).unwrap(), b"old");
    assert!(!r.join("b.pak").exists());
}
#[test]
fn drop_import_journal_failure_preserves_published_results_and_stops_remaining_items() {
    for failure in ["insert", "update"] {
        let (t, mut s, g, r) = setup();
        let f = s.lib.games[0].folders[0].id.clone();
        let mut paths = vec![];
        for name in ["a.pak", "b.pak", "c.pak"] {
            let p = t.path().join(name);
            fs::write(&p, name).unwrap();
            paths.push(p);
        }
        let sql = if failure == "insert" {
            "CREATE TRIGGER fail_import BEFORE INSERT ON file_imports WHEN NEW.source LIKE '%b.pak' BEGIN SELECT RAISE(FAIL,'test journal insert failure'); END;"
        } else {
            "CREATE TRIGGER fail_import BEFORE UPDATE ON file_imports WHEN NEW.status='done' BEGIN SELECT RAISE(FAIL,'test journal update failure'); END;"
        };
        s.conn.execute_batch(sql).unwrap();
        let out = s
            .request(json!({"action":"importFiles","gameId":g,"folderId":f,"paths":paths}))
            .unwrap();
        assert_eq!(out["results"][0]["success"], true);
        assert_eq!(out["results"][1]["success"], false);
        assert_eq!(out["results"][2]["success"], false);
        assert!(out["batchError"].as_str().unwrap().contains("操作记录"));
        assert_eq!(fs::read(r.join("a.pak")).unwrap(), b"a.pak");
        assert!(!r.join("b.pak").exists());
        assert!(!r.join("c.pak").exists());
    }
}
#[test]
fn drop_import_rejects_own_directory_ancestors_duplicates_and_unknown_targets() {
    let (t, mut s, g, r) = setup();
    let f = s.lib.games[0].folders[0].id.clone();
    let source = t.path().join("a.pak");
    fs::write(&source, "a").unwrap();
    assert!(s.request(json!({"action":"importFiles","gameId":g,"folderId":f,"expectedPath":t.path().join("old-root"),"paths":[source]})).unwrap_err().contains("配置已变化"));
    for paths in [
        vec![r.clone()],
        vec![t.path().to_path_buf()],
        vec![source.clone(), source.clone()],
    ] {
        assert!(s
            .request(json!({"action":"importFiles","gameId":g,"folderId":f,"paths":paths}))
            .is_err());
    }
    assert!(s
        .request(json!({"action":"importFiles","gameId":g,"folderId":"unknown","paths":[source]}))
        .is_err());
    assert!(fs::read_dir(r).unwrap().next().is_none());
}
#[cfg(windows)]
#[test]
fn drop_import_locked_file_reports_partial_result_cleans_stage_and_keeps_sources() {
    use std::os::windows::fs::OpenOptionsExt;
    let (t, mut s, g, r) = setup();
    let f = s.lib.games[0].folders[0].id.clone();
    let a = t.path().join("a.pak");
    let b = t.path().join("b.pak");
    fs::write(&a, "a").unwrap();
    fs::write(&b, "b").unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&b)
        .unwrap();
    let out = s
        .request(json!({"action":"importFiles","gameId":g,"folderId":f,"paths":[a,b]}))
        .unwrap();
    assert_eq!(out["results"][0]["success"], true);
    assert_eq!(out["results"][1]["success"], false);
    assert!(r.join("a.pak").exists());
    assert!(!r.join("b.pak").exists());
    assert!(!fs::read_dir(t.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".mod-manager-import-")));
    drop(lock);
    assert_eq!(fs::read(&b).unwrap(), b"b");
}
#[test]
fn interrupted_import_is_reported_without_deleting_or_publishing_copies() {
    let (t, s, _, r) = setup();
    let stage = t.path().join(".mod-manager-import-interrupted");
    fs::create_dir(&stage).unwrap();
    fs::write(stage.join("payload"), "partial").unwrap();
    s.conn.execute("INSERT INTO file_imports(id,source,target,stage,status) VALUES('test','source',?1,?2,'pending')",params![r.join("a.pak").to_string_lossy(),stage.to_string_lossy()]).unwrap();
    drop(s);
    let s = Service::new(t.path().join("data")).unwrap();
    assert!(s.warnings.iter().any(|w| w.contains("拖放导入中断")));
    assert!(!r.join("a.pak").exists());
    assert_eq!(fs::read(stage.join("payload")).unwrap(), b"partial");
}
#[cfg(windows)]
#[test]
fn drop_import_rejects_nested_junction_without_touching_external_files() {
    let (t, mut s, g, r) = setup();
    let f = s.lib.games[0].folders[0].id.clone();
    let external = t.path().join("external");
    let source = t.path().join("download");
    fs::create_dir(&external).unwrap();
    fs::create_dir(&source).unwrap();
    fs::write(external.join("keep.txt"), "keep").unwrap();
    assert!(std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(source.join("junction"))
        .arg(&external)
        .status()
        .unwrap()
        .success());
    assert!(s
        .request(json!({"action":"importFiles","gameId":g,"folderId":f,"paths":[source]}))
        .unwrap_err()
        .contains("链接"));
    assert_eq!(fs::read(external.join("keep.txt")).unwrap(), b"keep");
    assert!(fs::read_dir(r).unwrap().next().is_none());
}
#[test]
fn folder_restore_retains_contents_that_existed_when_deleted() {
    let (_t, mut s, _, r) = setup();
    fs::create_dir(r.join("bundle")).unwrap();
    fs::write(r.join("bundle/a.txt"), "a").unwrap();
    fs::write(r.join("bundle/missing.txt"), "old").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    fs::rename(
        r.join("bundle/missing.txt"),
        r.parent().unwrap().join("kept-outside.txt"),
    )
    .unwrap();
    let snap = s.request(json!({"action":"snapshot"})).unwrap();
    let m = snap["mods"][0].clone();
    let snap = s
        .request(
            json!({"action":"deleteMod","id":id,"confirmed":true,"expectedMembers":m["members"]}),
        )
        .unwrap();
    let snap = s
        .request(json!({"action":"restoreMod","operationId":snap["recentDeletions"][0]["id"]}))
        .unwrap();
    assert_eq!(snap["mods"][0]["id"], id);
    assert!(r.join("bundle/a.txt").exists());
    assert!(!r.join("bundle/missing.txt").exists());
}
#[test]
fn interrupted_restore_keeps_original_ids_and_can_finish_after_restart() {
    let (t, mut s, _, r) = setup();
    for ext in ["pak", "utoc", "ucas"] {
        fs::write(r.join(format!("a.{ext}")), ext).unwrap();
    }
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let m: Mod = serde_json::from_value(snap["mods"][0].clone()).unwrap();
    s.request(json!({"action":"editMod","id":m.id,"description":"crash-safe note"}))
        .unwrap();
    let recorded = s.lib.mods[0].clone();
    let snap=s.request(json!({"action":"deleteMod","id":m.id,"confirmed":true,"expectedMembers":snap["mods"][0]["members"]})).unwrap();
    let op = snap["recentDeletions"][0]["id"].clone();
    let items = restoration::find_recycled(&recorded.members).unwrap();
    s.lib.mods.push(recorded.clone());
    s.persist().unwrap();
    s.conn
        .execute(
            "UPDATE deletions SET status='restoring' WHERE id=?1",
            params![op.as_str().unwrap()],
        )
        .unwrap();
    no_replace_move(&items[0].path, &recorded.members[0].current_path).unwrap();
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    assert_eq!(s.lib.mods.iter().filter(|x| x.id == m.id).count(), 1);
    let snap = s
        .request(json!({"action":"restoreMod","operationId":op}))
        .unwrap();
    let restored = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == m.id)
        .unwrap();
    assert_eq!(restored["description"], "crash-safe note");
    assert_eq!(restored["members"].as_array().unwrap().len(), 3);
}
#[test]
fn restore_disabled_folder_keeps_cover_note_leaf_ownership_and_real_state() {
    let (_t, mut s, _, r) = setup();
    fs::create_dir(r.join("bundle")).unwrap();
    fs::write(r.join("bundle/a.txt"), "a").unwrap();
    fs::write(r.join("bundle/keep.txt.disabled"), "keep").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    let cover = r.parent().unwrap().join("cover.png");
    let image=STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=").unwrap();
    fs::write(&cover, &image).unwrap();
    s.request(json!({"action":"editMod","id":id,"managed":true,"description":"folder note","imagePath":cover})).unwrap();
    s.request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    let snap = s.request(json!({"action":"snapshot"})).unwrap();
    let m = snap["mods"][0].clone();
    let snap = s
        .request(
            json!({"action":"deleteMod","id":id,"confirmed":true,"expectedMembers":m["members"]}),
        )
        .unwrap();
    let snap = s
        .request(json!({"action":"restoreMod","operationId":snap["recentDeletions"][0]["id"]}))
        .unwrap();
    assert_eq!(snap["mods"][0]["state"], "disabled");
    assert_eq!(snap["mods"][0]["description"], "folder note");
    assert_eq!(snap["mods"][0]["preservedDisabledFiles"], 1);
    assert_eq!(
        fs::read(snap["mods"][0]["imagePath"].as_str().unwrap()).unwrap(),
        image
    );
    s.request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    assert!(r.join("bundle/a.txt").exists());
    assert!(r.join("bundle/keep.txt.disabled").exists());
    assert!(!r.join("bundle/keep.txt").exists());
}
#[test]
fn restore_refuses_changed_game_directory_and_forged_audit_paths() {
    let (t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), "a").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let m = snap["mods"][0].clone();
    let snap=s.request(json!({"action":"deleteMod","id":m["id"],"confirmed":true,"expectedMembers":m["members"]})).unwrap();
    let op = snap["recentDeletions"][0]["id"].clone();
    let other = t.path().join("other");
    fs::create_dir(&other).unwrap();
    s.request(json!({"action":"saveGame","game":{"id":g,"name":"changed","modDir":other,"strategy":"suffix"}})).unwrap();
    assert!(s
        .request(json!({"action":"restoreMod","operationId":op}))
        .is_err());
    assert!(!r.join("a.pak").exists());
    let mut forged = m.clone();
    forged["members"][0]["originalPath"] = json!(t.path().join("outside.pak"));
    forged["members"][0]["currentPath"] = forged["members"][0]["originalPath"].clone();
    s.conn
        .execute(
            "UPDATE deletions SET data=?1 WHERE id=?2",
            params![forged.to_string(), op.as_str().unwrap()],
        )
        .unwrap();
    assert!(s
        .request(json!({"action":"restoreMod","operationId":op}))
        .is_err());
    assert!(!t.path().join("outside.pak").exists());
}
#[test]
fn view_preferences_survive_restart_and_reject_oversized_data() {
    let (t, mut s, g, _) = setup();
    let view = json!({"gameId":g,"folderChoices":{},"scrolls":{"test":135}});
    assert_eq!(
        s.request(json!({"action":"saveView","view":view})).unwrap()["success"],
        true
    );
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["view"],
        view
    );
    assert!(s
        .request(json!({"action":"saveView","view":{"gameId":"x".repeat(20000)}}))
        .is_err());
}
#[test]
fn recycle_restore_preserves_metadata_and_rejects_conflict_then_survives_restart() {
    let (t, mut s, _, r) = setup();
    for ext in ["pak", "utoc", "ucas"] {
        fs::write(r.join(format!("bundle.{ext}")), ext).unwrap();
    }
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let m = snap["mods"][0].clone();
    s.request(json!({"action":"editMod","id":m["id"],"description":"恢复备注"}))
        .unwrap();
    let deleted=s.request(json!({"action":"deleteMod","id":m["id"],"confirmed":true,"expectedMembers":m["members"]})).unwrap();
    let operation = deleted["recentDeletions"][0]["id"].clone();
    assert!(operation.is_string());
    fs::write(r.join("bundle.pak"), "new conflicting file").unwrap();
    assert!(s
        .request(json!({"action":"restoreMod","operationId":operation}))
        .is_err());
    assert_eq!(
        fs::read_to_string(r.join("bundle.pak")).unwrap(),
        "new conflicting file"
    );
    fs::rename(r.join("bundle.pak"), r.join("conflict-kept.pak")).unwrap();
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    let restored = s
        .request(json!({"action":"restoreMod","operationId":operation}))
        .unwrap();
    let restored = restored["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == m["id"])
        .unwrap();
    assert_eq!(restored["description"], "恢复备注");
    assert_eq!(restored["members"].as_array().unwrap().len(), 3);
    for ext in ["pak", "utoc", "ucas"] {
        assert_eq!(
            fs::read_to_string(r.join(format!("bundle.{ext}"))).unwrap(),
            ext
        );
    }
    assert!(s
        .request(json!({"action":"restoreMod","operationId":operation}))
        .is_err());
}
#[test]
fn externally_disabled_only_folder_cannot_be_restored_by_manager() {
    let (_t, mut s, _, r) = setup();
    let folder = r.join("bundle");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("keep.txt.disabled"), "untouched").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    assert_eq!(snap["mods"][0]["state"], "disabled");
    s.request(json!({"action":"editMod","id":id,"managed":true}))
        .unwrap();
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], false);
    assert!(result["results"][0]["error"]
        .as_str()
        .unwrap()
        .contains("原本已禁用"));
    assert!(!folder.join("keep.txt").exists());
    assert_eq!(
        fs::read_to_string(folder.join("keep.txt.disabled")).unwrap(),
        "untouched"
    );
}
#[test]
fn tree_preserves_preexisting_disabled_files_across_restart_and_backup() {
    let (t, mut s, _, r) = setup();
    let folder = r.join("bundle");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("normal.txt"), "normal").unwrap();
    fs::write(folder.join("keep.txt.disabled"), "untouched").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"managed":true}))
        .unwrap();
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], true, "{result}");
    assert!(folder.join("normal.txt.disabled").exists());
    assert!(!folder.join("keep.txt.disabled.disabled").exists());
    let backup = t.path().join("protected.json");
    s.request(json!({"action":"export","path":backup})).unwrap();
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], true, "{result}");
    assert!(folder.join("normal.txt").exists());
    assert!(
        !folder.join("keep.txt").exists(),
        "never restore a preexisting disabled file"
    );
    assert_eq!(
        fs::read_to_string(folder.join("keep.txt.disabled")).unwrap(),
        "untouched"
    );
    let snap = s.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(snap["mods"][0]["state"], "enabled");
    assert_eq!(snap["mods"][0]["preservedDisabledFiles"], 1);
    s.request(json!({"action":"import","path":backup})).unwrap();
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], true, "{result}");
    fs::write(folder.join("added.txt.disabled"), "added externally").unwrap();
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], true, "{result}");
    assert!(!folder.join("keep.txt").exists());
    assert!(!folder.join("added.txt").exists());
}

#[test]
fn tree_does_not_restore_externally_disabled_tracked_file() {
    let (_t, mut s, _, r) = setup();
    let folder = r.join("bundle");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("normal.txt"), "normal").unwrap();
    fs::write(folder.join("external.txt"), "external").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"managed":true}))
        .unwrap();
    no_replace_move(
        &folder.join("external.txt"),
        &folder.join("external.txt.disabled"),
    )
    .unwrap();
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], true, "{result}");
    assert!(folder.join("external.txt.disabled").exists());
    assert!(!folder.join("external.txt").exists());
}

#[test]
fn legacy_tree_inventory_still_restores_after_upgrade() {
    let (t, mut s, _, r) = setup();
    let folder = r.join("bundle");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("normal.txt"), "normal").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"managed":true}))
        .unwrap();
    s.request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    let mut old = serde_json::to_value(&s.lib).unwrap();
    for leaf in old["mods"][0]["members"][0]["leaves"]
        .as_array_mut()
        .unwrap()
    {
        leaf.as_object_mut().unwrap().remove("disabledByManager");
    }
    s.conn
        .execute(
            "UPDATE library SET data=?1 WHERE id=1",
            params![old.to_string()],
        )
        .unwrap();
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], true, "{result}");
    assert!(folder.join("normal.txt").exists());
}
#[test]
fn image_preview_is_validated_and_does_not_create_files_or_change_library() {
    let (t, mut s, _, _) = setup();
    let image = t.path().join("preview.png");
    let bytes = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=").unwrap();
    fs::write(&image, &bytes).unwrap();
    let before = s.request(json!({"action":"snapshot"})).unwrap();
    let out = s
        .request(json!({"action":"previewImage","path":image}))
        .unwrap();
    assert_eq!(
        out["url"],
        format!("data:image/png;base64,{}", STANDARD.encode(&bytes))
    );
    assert!(!t.path().join("data/images").exists());
    fs::write(&image, b"invalid").unwrap();
    assert!(s
        .request(json!({"action":"previewImage","path":image}))
        .is_err());
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["games"],
        before["games"]
    );
}
#[test]
fn oversized_cover_backup_is_rejected_before_creating_unrestorable_file() {
    let (t, mut s, g, r) = setup();
    for n in 0..4 {
        fs::write(r.join(format!("mod{n}.pak")), b"mod").unwrap();
    }
    let snap = s.request(json!({"action":"scan","gameId":g})).unwrap();
    let image = t.path().join("large-cover.png");
    let mut bytes = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=").unwrap();
    bytes.resize(13 * 1024 * 1024, 0);
    fs::write(&image, bytes).unwrap();
    for m in snap["mods"].as_array().unwrap() {
        s.request(json!({"action":"editMod","id":m["id"],"imagePath":image}))
            .unwrap();
    }
    let backup = t.path().join("too-large.json");
    assert!(s.request(json!({"action":"export","path":backup})).is_err());
    assert!(!backup.exists());
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["mods"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}
#[test]
fn mod_cover_persists_backs_up_and_clears_without_touching_mod() {
    let (t, mut s, g, r) = setup();
    fs::write(r.join("dress.pak"), b"mod-content").unwrap();
    let snap = s.request(json!({"action":"scan","gameId":g})).unwrap();
    let id = snap["mods"][0]["id"].as_str().unwrap().to_owned();
    let image = t.path().join("cover.png");
    let bytes = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=").unwrap();
    fs::write(&image, &bytes).unwrap();
    let snap = s
        .request(json!({"action":"editMod","id":id,"imagePath":image}))
        .unwrap();
    let stored = PathBuf::from(
        snap["mods"][0]["imagePath"]
            .as_str()
            .expect("Mod cover must be stored"),
    );
    assert_ne!(stored, image);
    fs::remove_file(image).unwrap();
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["mods"][0]["imagePath"],
        json!(stored)
    );
    let backup = t.path().join("mod-cover.json");
    s.request(json!({"action":"export","path":backup})).unwrap();
    fs::remove_file(stored).unwrap();
    let restored = s.request(json!({"action":"import","path":backup})).unwrap();
    assert_eq!(
        fs::read(restored["mods"][0]["imagePath"].as_str().unwrap()).unwrap(),
        bytes
    );
    let cleared = s
        .request(json!({"action":"editMod","id":id,"imagePath":null}))
        .unwrap();
    assert!(cleared["mods"][0]["imagePath"].is_null());
    assert_eq!(fs::read(r.join("dress.pak")).unwrap(), b"mod-content");
}
#[test]
fn invalid_mod_cover_does_not_change_metadata() {
    let (t, mut s, g, r) = setup();
    fs::write(r.join("dress.pak"), b"mod").unwrap();
    let snap = s.request(json!({"action":"scan","gameId":g})).unwrap();
    let id = snap["mods"][0]["id"].as_str().unwrap();
    let image = t.path().join("bad.png");
    fs::write(&image, b"not an image").unwrap();
    assert!(s
        .request(json!({"action":"editMod","id":id,"name":"must not save","imagePath":image}))
        .is_err());
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["mods"][0]["name"],
        ""
    );
}
fn setup() -> (tempfile::TempDir, Service, String, PathBuf) {
    let temp = tempdir().unwrap();
    let root = temp.path().join("mods");
    fs::create_dir(&root).unwrap();
    // Windows hosted runners may expose TEMP through an 8.3 alias (RUNNER~1).
    // Fixtures that construct recovery records must use the same canonical
    // spelling as saveGame, just like records produced by the application.
    let root = fs::canonicalize(root).unwrap();
    let mut s = Service::new(temp.path().join("data")).unwrap();
    let out = s
        .request(
            json!({"action":"saveGame","game":{"name":"Test","modDir":root,"strategy":"suffix"}}),
        )
        .unwrap();
    let id = out["games"][0]["id"].as_str().unwrap().to_string();
    (temp, s, id, root)
}
#[test]
fn suffix_group_metadata_survives_restart() {
    let (t, mut s, g, r) = setup();
    for ext in ["pak", "utoc", "ucas"] {
        fs::write(r.join(format!("dress.{ext}")), b"data").unwrap();
    }
    let snap = s.request(json!({"action":"scan","gameId":g})).unwrap();
    assert_eq!(snap["mods"].as_array().unwrap().len(), 1);
    let id = snap["mods"][0]["id"].as_str().unwrap();
    s.request(
        json!({"action":"editMod","id":id,"name":"礼服","description":"备注","managed":true}),
    )
    .unwrap();
    let res = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(res["results"][0]["success"], true);
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    let snap = s.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(snap["mods"][0]["name"], "礼服");
    assert_eq!(snap["mods"][0]["state"], "disabled");
    s.request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    assert!(r.join("dress.pak").exists());
}
#[test]
fn collision_preflight_never_overwrites() {
    let (_t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), b"on").unwrap();
    fs::write(r.join("a.pak.disabled"), b"off").unwrap();
    let x = s.request(json!({"action":"scan","gameId":g})).unwrap();
    let id = x["mods"][0]["id"].clone();
    let z = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(z["results"][0]["success"], false);
    assert_eq!(fs::read(r.join("a.pak")).unwrap(), b"on");
    assert_eq!(fs::read(r.join("a.pak.disabled")).unwrap(), b"off");
}
#[test]
fn missing_metadata_and_unknown_candidate() {
    let (_t, mut s, g, r) = setup();
    fs::write(r.join("notes.txt"), b"hello").unwrap();
    let x = s.request(json!({"action":"scan","gameId":g})).unwrap();
    assert_eq!(x["mods"][0]["managed"], false);
    let id = x["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"name":"保留","description":"long"}))
        .unwrap();
    fs::remove_file(r.join("notes.txt")).unwrap();
    let x = s.request(json!({"action":"scan"})).unwrap();
    assert_eq!(x["mods"][0]["state"], "missing");
    assert_eq!(x["mods"][0]["name"], "保留");
}
#[test]
fn move_folder_group_and_backup() {
    let (t, mut s, g, r) = setup();
    fs::create_dir(r.join("folder")).unwrap();
    fs::write(r.join("folder").join("x.txt"), b"x").unwrap();
    s.request(
        json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":r,"strategy":"move"}}),
    )
    .unwrap();
    let x = s.request(json!({"action":"scan"})).unwrap();
    let id = x["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"managed":true}))
        .unwrap();
    let z = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(z["results"][0]["success"], true);
    assert!(!r.join("folder").exists());
    s.request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    assert!(r.join("folder/x.txt").exists());
    let b = t.path().join("backup.json");
    s.request(json!({"action":"export","path":b})).unwrap();
    s.request(json!({"action":"import","path":b})).unwrap();
    assert!(r.join("folder/x.txt").exists());
}
#[test]
fn manual_group_detach_delete_only_metadata() {
    let (_t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    fs::write(r.join("b.pak"), b"b").unwrap();
    let x = s.request(json!({"action":"scan"})).unwrap();
    let ids: Vec<_> = x["mods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].clone())
        .collect();
    let x = s
        .request(json!({"action":"group","ids":ids,"name":"group","description":""}))
        .unwrap();
    assert_eq!(x["mods"].as_array().unwrap().len(), 1);
    let id = x["mods"][0]["id"].clone();
    s.request(json!({"action":"ungroup","id":id})).unwrap();
    s.request(json!({"action":"deleteGame","id":g})).unwrap();
    assert!(r.join("a.pak").exists());
}
#[test]
fn recovery_rolls_back_durable_journal() {
    let (t, mut s, _g, r) = setup();
    let a = r.join("a.pak");
    let b = r.join("a.pak.disabled");
    fs::write(&a, b"a").unwrap();
    s.request(json!({"action":"scan"})).unwrap();
    let plan = vec![Step {
        from: a.clone(),
        to: b.clone(),
    }];
    s.conn
        .execute(
            "INSERT INTO journals(id,plan) VALUES (?1,?2)",
            rusqlite::params!["test", serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
    fs::rename(&a, &b).unwrap();
    drop(s);
    let _s = Service::new(t.path().join("data")).unwrap();
    assert!(a.exists());
    assert!(!b.exists());
}

#[test]
fn change_directory_preserves_old_notes_and_backup() {
    let (t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), b"old").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"name":"old notes"}))
        .unwrap();
    let newroot = t.path().join("newmods");
    fs::create_dir(&newroot).unwrap();
    fs::write(newroot.join("a.pak"), b"new").unwrap();
    let snap=s.request(json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":newroot,"strategy":"suffix"}})).unwrap();
    assert_eq!(snap["mods"].as_array().unwrap().len(), 2);
    let old = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .unwrap();
    assert_eq!(old["state"], "missing");
    assert_eq!(old["managed"], false);
    let res = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(res["results"][0]["success"], false);
    let b = t.path().join("change.json");
    s.request(json!({"action":"export","path":b})).unwrap();
    s.request(json!({"action":"import","path":b})).unwrap();
    assert!(r.join("a.pak").exists());
    assert!(newroot.join("a.pak").exists());
}
#[test]
fn invalid_backup_does_not_touch_library() {
    let (t, mut s, _g, _r) = setup();
    let b = t.path().join("bad.json");
    fs::write(&b, b"{\"format\":\"mod-manager\",\"version\":99}").unwrap();
    assert!(s.request(json!({"action":"import","path":b})).is_err());
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["games"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn disabled_suffix_is_exact_and_never_duplicated() {
    let (_t, mut s, _g, r) = setup();
    fs::write(r.join("a.pak.disabled"), b"a").unwrap();
    let x = s.request(json!({"action":"scan"})).unwrap();
    let id = x["mods"][0]["id"].clone();
    assert_eq!(x["mods"][0]["state"], "disabled");
    s.request(json!({"action":"toggle","ids":[id.clone()],"enabled":false}))
        .unwrap();
    assert!(!r.join("a.pak.disabled.disabled").exists());
    s.request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    assert!(r.join("a.pak").exists());
}
#[test]
fn untrusted_recovery_plan_is_blocked() {
    let (t, s, _g, _r) = setup();
    let a = t.path().join("not-a-mod");
    let b = t.path().join("other");
    fs::write(&b, b"keep").unwrap();
    let plan = vec![Step {
        from: a.clone(),
        to: b.clone(),
    }];
    s.conn
        .execute(
            "INSERT INTO journals(id,plan) VALUES(?1,?2)",
            params!["bad", serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    assert!(!a.exists());
    assert!(b.exists());
    assert!(
        !s.request(json!({"action":"snapshot"})).unwrap()["warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn automatic_siblings_arriving_later_keep_name() {
    let (_t, mut s, _g, r) = setup();
    fs::write(r.join("dress.pak"), b"a").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"name":"衣服","description":"notes"}))
        .unwrap();
    fs::write(r.join("dress.ucas"), b"a").unwrap();
    fs::write(r.join("dress.utoc"), b"a").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    assert_eq!(snap["mods"].as_array().unwrap().len(), 1);
    assert_eq!(snap["mods"][0]["members"].as_array().unwrap().len(), 3);
    assert_eq!(snap["mods"][0]["name"], "衣服");
}
#[cfg(windows)]
#[test]
fn locked_member_rolls_back_completed_members() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_t, mut s, _g, r) = setup();
    for ext in ["pak", "utoc"] {
        fs::write(r.join(format!("dress.{ext}")), b"bytes").unwrap();
    }
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    let _lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(r.join("dress.utoc"))
        .unwrap();
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], false);
    assert!(result["results"][0]["error"]
        .as_str()
        .unwrap()
        .contains("已恢复"));
    assert_eq!(fs::read(r.join("dress.pak")).unwrap(), b"bytes");
    assert!(r.join("dress.pak").exists());
    assert!(!r.join("dress.pak.disabled").exists());
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["mods"][0]["state"],
        "enabled"
    );
}
#[test]
fn member_reassignment_preserves_exclusive_ownership() {
    let (_t, mut s, _g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    fs::write(r.join("b.pak"), b"b").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    let memberids: Vec<_> = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["members"][0]["id"].clone())
        .collect();
    let snap = s
        .request(json!({"action":"setMembers","id":id,"memberIds":memberids}))
        .unwrap();
    assert_eq!(snap["mods"].as_array().unwrap().len(), 1);
    let keep = snap["mods"][0]["members"][0]["id"].clone();
    let snap = s
        .request(json!({"action":"setMembers","id":id,"memberIds":[keep]}))
        .unwrap();
    assert_eq!(snap["mods"].as_array().unwrap().len(), 2);
    assert_eq!(
        snap["mods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["members"].as_array().unwrap().len())
            .sum::<usize>(),
        2
    );
}
#[cfg(windows)]
#[test]
fn unresolved_recovery_stays_error_until_restart() {
    use std::os::windows::fs::OpenOptionsExt;
    let (t, mut s, _g, r) = setup();
    for ext in ["pak", "utoc"] {
        fs::write(r.join(format!("dress.{ext}")), b"bytes").unwrap();
    }
    s.request(json!({"action":"scan"})).unwrap();
    let a = r.join("dress.pak");
    let b = r.join("dress.pak.disabled");
    let plan = vec![
        Step {
            from: a.clone(),
            to: b.clone(),
        },
        Step {
            from: r.join("dress.utoc"),
            to: r.join("dress.utoc.disabled"),
        },
    ];
    s.conn
        .execute(
            "INSERT INTO journals(id,plan) VALUES(?1,?2)",
            params!["partial", serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
    fs::rename(&a, &b).unwrap();
    drop(s);
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&b)
        .unwrap();
    let mut s = Service::new(t.path().join("data")).unwrap();
    for _ in 0..2 {
        assert_eq!(
            s.request(json!({"action":"snapshot"})).unwrap()["mods"][0]["state"],
            "error"
        );
    }
    drop(lock);
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["mods"][0]["state"],
        "enabled"
    );
    assert!(a.exists());
    assert!(!b.exists());
}
#[test]
fn bulk_partial_results_many_long_names_empty_root() {
    let (_t, mut s, _g, r) = setup();
    assert!(s.request(json!({"action":"scan"})).unwrap()["mods"]
        .as_array()
        .unwrap()
        .is_empty());
    for n in 0..100 {
        fs::write(
            r.join(format!("{n:03}_{}.pak", "long".repeat(30))),
            b"content",
        )
        .unwrap();
    }
    fs::write(r.join("unknown.txt"), b"note").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    assert_eq!(snap["mods"].as_array().unwrap().len(), 101);
    let ids: Vec<_> = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].clone())
        .collect();
    let result = s
        .request(json!({"action":"toggle","ids":ids,"enabled":false}))
        .unwrap();
    let results = result["results"].as_array().unwrap();
    assert_eq!(results.iter().filter(|r| r["success"] == true).count(), 100);
    assert_eq!(results.iter().filter(|r| r["success"] == false).count(), 1);
    assert!(r.join("unknown.txt").exists());
}
#[test]
fn custom_image_bytes_survive_export_import() {
    let (t, mut s, g, r) = setup();
    let image = t.path().join("icon.png");
    let bytes=STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=").unwrap();
    fs::write(&image, &bytes).unwrap();
    let snap=s.request(json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":r,"strategy":"suffix","imagePath":image}})).unwrap();
    let stored = PathBuf::from(snap["games"][0]["imagePath"].as_str().unwrap());
    assert_ne!(stored, image);
    assert_eq!(fs::read(&stored).unwrap(), bytes);
    fs::remove_file(&image).unwrap();
    let backup = t.path().join("images.json");
    s.request(json!({"action":"export","path":backup})).unwrap();
    fs::remove_file(&stored).unwrap();
    let snap = s.request(json!({"action":"import","path":backup})).unwrap();
    let restored = PathBuf::from(snap["games"][0]["imagePath"].as_str().unwrap());
    assert_eq!(fs::read(restored).unwrap(), bytes);
}
#[cfg(windows)]
#[test]
fn junction_source_and_directory_descendants_are_blocked() {
    let (t, mut s, g, r) = setup();
    let external = t.path().join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("keep.txt"), b"keep").unwrap();
    let folder = r.join("folder");
    fs::create_dir(&folder).unwrap();
    let link = folder.join("junction");
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&link)
        .arg(&external)
        .status()
        .unwrap();
    assert!(status.success(), "Windows junction creation failed");
    s.request(
        json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":r,"strategy":"move"}}),
    )
    .unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"managed":true}))
        .unwrap();
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], false);
    assert_eq!(fs::read(external.join("keep.txt")).unwrap(), b"keep");
    fs::remove_dir(&link).unwrap();
}

#[test]
fn failed_game_edit_leaves_old_root_active() {
    let (t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), b"old").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    let other = t.path().join("othermods");
    fs::create_dir(&other).unwrap();
    let result=s.request(json!({"action":"saveGame","game":{"id":g,"name":"Changed","modDir":other,"strategy":"suffix","exePath":t.path().join("missing.exe")}}));
    assert!(result.is_err());
    let snap = s.request(json!({"action":"scan"})).unwrap();
    assert_eq!(snap["games"][0]["name"], "Test");
    assert_eq!(snap["mods"][0]["id"], id);
    assert_eq!(snap["mods"][0]["retired"], false);
    assert_eq!(snap["mods"][0]["state"], "enabled");
    assert_eq!(
        s.request(json!({"action":"toggle","ids":[id],"enabled":false}))
            .unwrap()["results"][0]["success"],
        true
    );
}
#[test]
fn root_a_b_a_archives_are_restart_and_backup_safe() {
    let (t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), b"old").unwrap();
    s.request(json!({"action":"scan"})).unwrap();
    let other = t.path().join("othermods");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("a.pak"), b"other").unwrap();
    for dir in [&other, &r] {
        s.request(json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":dir,"strategy":"suffix"}})).unwrap();
    }
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    let snap = s.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(
        snap["mods"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["retired"] == false)
            .count(),
        1
    );
    assert_eq!(
        snap["mods"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["retired"] == true)
            .count(),
        2
    );
    let backup = t.path().join("archive.json");
    s.request(json!({"action":"export","path":backup})).unwrap();
    s.request(json!({"action":"import","path":backup})).unwrap();
    assert!(r.join("a.pak").exists());
    assert!(other.join("a.pak").exists());
}
#[test]
fn delete_disabled_move_game_preserves_restore_mapping() {
    let (_t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    s.request(
        json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":r,"strategy":"move"}}),
    )
    .unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    s.request(json!({"action":"toggle","ids":[id.clone()],"enabled":false}))
        .unwrap();
    assert!(s.request(json!({"action":"deleteGame","id":g})).is_err());
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["games"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.request(json!({"action":"toggle","ids":[id],"enabled":true}))
            .unwrap()["results"][0]["success"],
        true
    );
    assert!(r.join("a.pak").exists());
    s.request(json!({"action":"deleteGame","id":g})).unwrap();
    assert!(r.join("a.pak").exists());
}
#[test]
fn pending_journal_blocks_import_and_metadata_delete() {
    let (t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    s.request(json!({"action":"scan"})).unwrap();
    let backup = t.path().join("pending.json");
    s.request(json!({"action":"export","path":backup})).unwrap();
    let plan = vec![Step {
        from: r.join("a.pak"),
        to: r.join("a.pak.disabled"),
    }];
    s.conn
        .execute(
            "INSERT INTO journals(id,plan) VALUES(?1,?2)",
            params!["pending", serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
    assert!(s.request(json!({"action":"import","path":backup})).is_err());
    assert!(s.request(json!({"action":"deleteGame","id":g})).is_err());
    assert!(r.join("a.pak").exists());
}
#[test]
fn same_path_replacement_does_not_reuse_metadata_for_toggle() {
    let (_t, mut s, _g, r) = setup();
    let file = r.join("a.pak");
    fs::write(&file, b"old mod").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    s.request(
        json!({"action":"editMod","id":id,"name":"old notes","description":"old description"}),
    )
    .unwrap();
    fs::remove_file(&file).unwrap();
    fs::write(&file, b"unrelated replacement").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    assert_eq!(snap["mods"][0]["state"], "error");
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], false);
    assert_eq!(fs::read(file).unwrap(), b"unrelated replacement");
    assert!(!snap["warnings"].as_array().unwrap().is_empty());
}
#[test]
fn crafted_game_identifier_cannot_escape_or_create_unknown_id() {
    let (t, mut s, _g, r) = setup();
    for id in ["../escape".to_string(), uid()] {
        assert!(s.request(json!({"action":"saveGame","game":{"id":id,"name":"Bad","modDir":r,"strategy":"move"}})).is_err());
    }
    assert!(!t.path().join("escape").exists());
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["games"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn import_older_strategy_while_disabled_is_refused() {
    let (t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    s.request(json!({"action":"scan"})).unwrap();
    let backup = t.path().join("old.json");
    s.request(json!({"action":"export","path":backup})).unwrap();
    let snap = s
        .request(
            json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":r,"strategy":"move"}}),
        )
        .unwrap();
    let id = snap["mods"][0]["id"].clone();
    assert_eq!(
        s.request(json!({"action":"toggle","ids":[id.clone()],"enabled":false}))
            .unwrap()["results"][0]["success"],
        true
    );
    assert!(s.request(json!({"action":"import","path":backup})).is_err());
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["games"][0]["strategy"],
        "move"
    );
    assert_eq!(
        s.request(json!({"action":"toggle","ids":[id],"enabled":true}))
            .unwrap()["results"][0]["success"],
        true
    );
    assert!(r.join("a.pak").exists());
}
#[test]
fn automatic_recovery_defers_while_configured_exe_runs() {
    let (t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    s.request(json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":r,"strategy":"suffix","exePath":std::env::current_exe().unwrap()}})).unwrap();
    let a = r.join("a.pak");
    let b = r.join("a.pak.disabled");
    let plan = vec![Step {
        from: a.clone(),
        to: b.clone(),
    }];
    s.conn
        .execute(
            "INSERT INTO journals(id,plan) VALUES(?1,?2)",
            params!["running", serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
    fs::rename(&a, &b).unwrap();
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    let snap = s.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(snap["mods"][0]["state"], "error");
    assert!(b.exists());
    assert!(!a.exists());
    assert!(!snap["warnings"].as_array().unwrap().is_empty());
}
#[test]
fn missing_group_cannot_switch_strategy_with_surviving_disabled_member() {
    let (_t, mut s, g, r) = setup();
    for ext in ["pak", "utoc"] {
        fs::write(r.join(format!("dress.{ext}")), b"a").unwrap();
    }
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    s.request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    fs::remove_file(r.join("dress.utoc.disabled")).unwrap();
    assert_eq!(
        s.request(json!({"action":"scan"})).unwrap()["mods"][0]["state"],
        "missing"
    );
    assert!(s
        .request(
            json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":r,"strategy":"move"}})
        )
        .is_err());
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["games"][0]["strategy"],
        "suffix"
    );
    assert!(r.join("dress.pak.disabled").exists());
}
#[test]
fn pending_healthy_journal_blocks_new_toggle_without_movement() {
    let (_t, mut s, _g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    let plan = vec![Step {
        from: r.join("a.pak"),
        to: r.join("a.pak.disabled"),
    }];
    s.conn
        .execute(
            "INSERT INTO journals(id,plan,status) VALUES(?1,?2,'pending')",
            params!["pending-healthy", serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["mods"][0]["state"],
        "enabled"
    );
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], false);
    assert_eq!(fs::read(r.join("a.pak")).unwrap(), b"a");
    assert!(!r.join("a.pak.disabled").exists());
}
#[test]
fn active_identity_controls_recovery_after_historical_root_replacement() {
    let (t, mut s, g, r) = setup();
    let a = r.join("a.pak");
    let b = r.join("a.pak.disabled");
    fs::write(&a, b"historical bytes").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let historical = snap["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":historical,"name":"historical notes"}))
        .unwrap();
    let other = t.path().join("other-root");
    fs::create_dir(&other).unwrap();
    s.request(json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":other,"strategy":"suffix"}})).unwrap();
    fs::remove_file(&a).unwrap();
    fs::write(&a, b"replacement bytes").unwrap();
    let snap=s.request(json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":r,"strategy":"suffix"}})).unwrap();
    let active = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["retired"] == false)
        .unwrap()["id"]
        .clone();
    assert_ne!(historical, active);
    let plan = vec![Step {
        from: a.clone(),
        to: b.clone(),
    }];
    s.conn
        .execute(
            "INSERT INTO journals(id,plan,status) VALUES(?1,?2,'pending')",
            params!["active-recovery", serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
    fs::rename(&a, &b).unwrap();
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    let snap = s.request(json!({"action":"snapshot"})).unwrap();
    let current = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == active)
        .unwrap();
    assert_eq!(current["state"], "enabled");
    assert_eq!(current["name"], "");
    let archived = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == historical)
        .unwrap();
    assert_eq!(archived["name"], "historical notes");
    assert_eq!(archived["state"], "missing");
    assert_eq!(fs::read(&a).unwrap(), b"replacement bytes");
    assert!(!b.exists());
    assert_eq!(
        s.conn
            .query_row("SELECT COUNT(*) FROM journals", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn archived_record_notes_edit_but_management_activation_is_rejected() {
    let (t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    let other = t.path().join("othermods");
    fs::create_dir(&other).unwrap();
    s.request(json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":other,"strategy":"suffix"}})).unwrap();
    s.request(json!({"action":"editMod","id":id,"name":"归档备注","description":"保留历史说明"}))
        .unwrap();
    assert!(s
        .request(json!({"action":"editMod","id":id,"managed":true}))
        .is_err());
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    let snap = s.request(json!({"action":"snapshot"})).unwrap();
    let archived = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .unwrap();
    assert_eq!(archived["managed"], false);
    assert_eq!(archived["state"], "missing");
    assert_eq!(archived["name"], "归档备注");
    assert_eq!(archived["description"], "保留历史说明");
    assert!(r.join("a.pak").exists());
}
#[test]
fn running_exe_alias_is_canonicalized_and_blocks_toggle() {
    let (_t, mut s, g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    let executable = std::env::current_exe().unwrap();
    let alias = executable
        .parent()
        .unwrap()
        .join(".")
        .join(executable.file_name().unwrap());
    let snap=s.request(json!({"action":"saveGame","game":{"id":g,"name":"Test","modDir":r,"strategy":"suffix","exePath":alias}})).unwrap();
    assert_eq!(
        PathBuf::from(snap["games"][0]["exePath"].as_str().unwrap()),
        fs::canonicalize(executable).unwrap()
    );
    let id = snap["mods"][0]["id"].clone();
    let result = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], false);
    assert!(result["results"][0]["error"]
        .as_str()
        .unwrap()
        .contains("游戏正在运行"));
    assert!(r.join("a.pak").exists());
    assert!(!r.join("a.pak.disabled").exists());
}
#[test]
fn explicit_replacement_confirmation_preserves_notes_and_allows_real_toggle() {
    let (_t, mut s, _g, r) = setup();
    let file = r.join("a.pak");
    fs::write(&file, b"original").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    let member = snap["mods"][0]["members"][0]["id"].clone();
    s.request(
        json!({"action":"editMod","id":id,"name":"已有中文名称","description":"用户保留的备注"}),
    )
    .unwrap();
    fs::remove_file(&file).unwrap();
    fs::write(&file, b"replacement bytes").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    assert_eq!(snap["mods"][0]["identityChanged"], true);
    assert_eq!(snap["mods"][0]["state"], "error");
    assert_eq!(
        s.request(json!({"action":"toggle","ids":[id.clone()],"enabled":false}))
            .unwrap()["results"][0]["success"],
        false
    );
    let snap = s
        .request(json!({"action":"acceptReplacement","id":id,"confirmed":true}))
        .unwrap();
    assert_eq!(snap["mods"][0]["identityChanged"], false);
    assert_eq!(snap["mods"][0]["state"], "enabled");
    assert_eq!(snap["mods"][0]["id"], id);
    assert_eq!(snap["mods"][0]["members"][0]["id"], member);
    assert_eq!(snap["mods"][0]["name"], "已有中文名称");
    assert_eq!(snap["mods"][0]["description"], "用户保留的备注");
    assert_eq!(fs::read(&file).unwrap(), b"replacement bytes");
    assert!(!r.join("a.pak.disabled").exists());
    assert_eq!(
        s.request(json!({"action":"toggle","ids":[id.clone()],"enabled":false}))
            .unwrap()["results"][0]["success"],
        true
    );
    assert_eq!(
        fs::read(r.join("a.pak.disabled")).unwrap(),
        b"replacement bytes"
    );
    assert_eq!(
        s.request(json!({"action":"toggle","ids":[id],"enabled":true}))
            .unwrap()["results"][0]["success"],
        true
    );
    assert_eq!(fs::read(file).unwrap(), b"replacement bytes");
}
#[test]
fn replacement_acceptance_refuses_unconfirmed_conflicting_missing_or_changed_kind_atomically() {
    let (_t, mut s, _g, r) = setup();
    for ext in ["pak", "utoc"] {
        fs::write(r.join(format!("dress.{ext}")), b"old").unwrap();
    }
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    let before: Vec<_> = s.lib.mods[0]
        .members
        .iter()
        .map(|f| f.disk_identity.clone())
        .collect();
    fs::remove_file(r.join("dress.pak")).unwrap();
    fs::write(r.join("dress.pak"), b"new").unwrap();
    s.request(json!({"action":"scan"})).unwrap();
    for confirmation in [Value::Null, json!(false), json!("true")] {
        assert!(s
            .request(json!({"action":"acceptReplacement","id":id,"confirmed":confirmation}))
            .is_err());
    }
    assert!(s
        .request(json!({"action":"acceptReplacement","id":uid(),"confirmed":true}))
        .is_err());
    fs::write(r.join("dress.pak.disabled"), b"collision").unwrap();
    assert!(s
        .request(json!({"action":"acceptReplacement","id":id,"confirmed":true}))
        .is_err());
    fs::remove_file(r.join("dress.pak.disabled")).unwrap();
    fs::remove_file(r.join("dress.utoc")).unwrap();
    assert!(s
        .request(json!({"action":"acceptReplacement","id":id,"confirmed":true}))
        .is_err());
    fs::create_dir(r.join("dress.utoc")).unwrap();
    assert!(s
        .request(json!({"action":"acceptReplacement","id":id,"confirmed":true}))
        .is_err());
    fs::remove_dir(r.join("dress.utoc")).unwrap();
    fs::write(r.join("dress.utoc"), b"new-member").unwrap();
    let plan = vec![Step {
        from: r.join("dress.pak"),
        to: r.join("dress.pak.disabled"),
    }];
    s.conn
        .execute(
            "INSERT INTO journals(id,plan) VALUES(?1,?2)",
            params!["pending-accept", serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
    assert!(s
        .request(json!({"action":"acceptReplacement","id":id,"confirmed":true}))
        .is_err());
    assert_eq!(
        s.lib.mods[0]
            .members
            .iter()
            .map(|f| f.disk_identity.clone())
            .collect::<Vec<_>>(),
        before
    );
    assert_eq!(fs::read(r.join("dress.pak")).unwrap(), b"new");
    assert!(!r.join("dress.pak.disabled").exists());
}

#[test]
fn multi_folder_same_names_toggle_independently_and_restart() {
    let t = tempdir().unwrap();
    let a = t.path().join("mods");
    let b = t.path().join("gameplay");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("same.pak"), b"appearance").unwrap();
    fs::write(b.join("same.pak"), b"gameplay").unwrap();
    let data = t.path().join("data");
    let mut s = Service::new(data.clone()).unwrap();
    let snap=s.request(json!({"action":"saveGame","game":{"name":"剑星","strategy":"move","folders":[{"name":"外观","path":a},{"name":"玩法","path":b}]}})).unwrap();
    let game = snap["games"][0]["id"].clone();
    let folders = snap["games"][0]["folders"].as_array().unwrap();
    assert_eq!(folders.len(), 2);
    assert_ne!(folders[0]["id"], folders[1]["id"]);
    let first = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["folderId"] == folders[0]["id"])
        .unwrap()
        .clone();
    let second = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["folderId"] == folders[1]["id"])
        .unwrap()
        .clone();
    s.request(json!({"action":"editMod","id":first["id"],"name":"外观备注"}))
        .unwrap();
    let result = s
        .request(json!({"action":"toggle","ids":[first["id"]],"enabled":false}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], true);
    assert!(!a.join("same.pak").exists());
    assert_eq!(fs::read(b.join("same.pak")).unwrap(), b"gameplay");
    let reordered = json!([folders[1].clone(), folders[0].clone()]);
    s.request(json!({"action":"saveGame","game":{"id":game,"name":"剑星","strategy":"move","folders":reordered}})).unwrap();
    drop(s);
    let mut s = Service::new(data).unwrap();
    let snapshot = s.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(
        snapshot["mods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == first["id"])
            .unwrap()["state"],
        "disabled"
    );
    let result = s
        .request(json!({"action":"toggle","ids":[first["id"],second["id"]],"enabled":true}))
        .unwrap();
    assert!(result["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["success"] == true));
    assert_eq!(fs::read(a.join("same.pak")).unwrap(), b"appearance");
    assert_eq!(fs::read(b.join("same.pak")).unwrap(), b"gameplay");
}

#[test]
fn folder_rename_keeps_identity_and_removal_guards_disabled_members() {
    let (t, mut s, g, a) = setup();
    let b = t.path().join("shaders");
    fs::create_dir(&b).unwrap();
    fs::write(b.join("shader.pak"), b"shader").unwrap();
    let mut folders = s.request(json!({"action":"snapshot"})).unwrap()["games"][0]["folders"]
        .as_array()
        .unwrap()
        .clone();
    folders.push(json!({"name":"光影","path":b}));
    let snap=s.request(json!({"action":"saveGame","game":{"id":g,"name":"Minecraft","strategy":"suffix","folders":folders}})).unwrap();
    let mut folders = snap["games"][0]["folders"].as_array().unwrap().clone();
    let fid = folders[1]["id"].clone();
    let id = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["folderId"] == fid)
        .unwrap()["id"]
        .clone();
    s.request(json!({"action":"editMod","id":id,"name":"中文光影","description":"保留备注"}))
        .unwrap();
    s.request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    folders[1]["name"] = json!("光影包");
    s.request(json!({"action":"saveGame","game":{"id":g,"name":"Minecraft","strategy":"suffix","folders":folders}})).unwrap();
    assert!(s.request(json!({"action":"saveGame","game":{"id":g,"name":"Minecraft","strategy":"suffix","folders":[folders[0]]}})).is_err());
    s.request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    let snap=s.request(json!({"action":"saveGame","game":{"id":g,"name":"Minecraft","strategy":"suffix","folders":[folders[0]]}})).unwrap();
    let history = snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .unwrap();
    assert_eq!(history["retired"], true);
    assert_eq!(history["name"], "中文光影");
    assert!(a.is_dir());
    assert_eq!(fs::read(b.join("shader.pak")).unwrap(), b"shader");
}

#[test]
fn overlapping_duplicate_roots_and_cross_folder_groups_are_blocked() {
    let (t, mut s, g, a) = setup();
    let nested = a.join("nested");
    let b = t.path().join("resources");
    fs::create_dir(&nested).unwrap();
    fs::create_dir(&b).unwrap();
    for folders in [
        json!([{"name":"A","path":a},{"name":"B","path":nested}]),
        json!([{"name":"A","path":a},{"name":"B","path":a}]),
    ] {
        assert!(s
            .request(json!({"action":"saveGame","game":{"id":g,"name":"Test","folders":folders}}))
            .is_err());
    }
    fs::write(a.join("a.pak"), b"a").unwrap();
    fs::write(b.join("b.pak"), b"b").unwrap();
    let snap=s.request(json!({"action":"saveGame","game":{"id":g,"name":"Test","folders":[{"name":"A","path":a},{"name":"B","path":b}]}})).unwrap();
    let mods = snap["mods"].as_array().unwrap();
    let one = mods.iter().find(|m| basename_test(m) == "a.pak").unwrap();
    let two = mods.iter().find(|m| basename_test(m) == "b.pak").unwrap();
    assert!(s
        .request(json!({"action":"group","ids":[one["id"],two["id"]]}))
        .is_err());
    assert!(s.request(json!({"action":"setMembers","id":one["id"],"memberIds":[one["members"][0]["id"],two["members"][0]["id"]]})).is_err());
    assert!(a.join("a.pak").exists());
    assert!(b.join("b.pak").exists());
}
fn basename_test(m: &Value) -> String {
    Path::new(m["members"][0]["originalPath"].as_str().unwrap())
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into()
}

#[test]
fn multi_folder_backup_roundtrip_and_legacy_migration() {
    let (t, mut s, g, a) = setup();
    let b = t.path().join("textures");
    fs::create_dir(&b).unwrap();
    fs::write(a.join("a.pak"), b"a").unwrap();
    fs::write(b.join("b.pak"), b"b").unwrap();
    s.request(json!({"action":"saveGame","game":{"id":g,"name":"Test","strategy":"move","folders":[{"name":"MOD","path":a},{"name":"材质包","path":b}]}})).unwrap();
    let file = t.path().join("backup.json");
    s.request(json!({"action":"export","path":file})).unwrap();
    let backup: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(backup["version"], 3);
    let mut restored = Service::new(t.path().join("restored")).unwrap();
    restored
        .request(json!({"action":"import","path":file}))
        .unwrap();
    let snap = restored.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(snap["games"][0]["folders"].as_array().unwrap().len(), 2);
    assert_eq!(snap["mods"].as_array().unwrap().len(), 2);
    let (_legacy, service, _gid, _root) = setup();
    let mut lib = serde_json::to_value(&service.lib).unwrap();
    for game in lib["games"].as_array_mut().unwrap() {
        game.as_object_mut().unwrap().remove("folders");
    }
    service
        .conn
        .execute(
            "UPDATE library SET data=?1 WHERE id=1",
            params![lib.to_string()],
        )
        .unwrap();
    let data = service.data_dir.clone();
    drop(service);
    let mut service = Service::new(data).unwrap();
    let snap = service.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(snap["games"][0]["folders"][0]["id"], snap["games"][0]["id"]);
    service.request(json!({"action":"saveGame","game":{"id":_gid,"name":"Legacy","modDir":_root,"strategy":"move"}})).unwrap();
    fs::write(_root.join("legacy.pak"), b"old-version-content").unwrap();
    let snap = service.request(json!({"action":"scan"})).unwrap();
    let mid = snap["mods"][0]["id"].clone();
    service
        .request(json!({"action":"toggle","ids":[mid],"enabled":false}))
        .unwrap();
    let legacy_file = _legacy.path().join("legacy-v1.json");
    service
        .request(json!({"action":"export","path":legacy_file}))
        .unwrap();
    let mut legacy: Value = serde_json::from_slice(&fs::read(&legacy_file).unwrap()).unwrap();
    legacy["version"] = json!(1);
    for g in legacy["library"]["games"].as_array_mut().unwrap() {
        g.as_object_mut().unwrap().remove("folders");
    }
    for m in legacy["library"]["mods"].as_array_mut().unwrap() {
        m.as_object_mut().unwrap().remove("folderId");
    }
    fs::write(&legacy_file, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let mut restored = Service::new(_legacy.path().join("v1-restored")).unwrap();
    let snap = restored
        .request(json!({"action":"import","path":legacy_file}))
        .unwrap();
    assert_eq!(snap["mods"][0]["state"], "disabled");
    let result = restored
        .request(json!({"action":"toggle","ids":[mid],"enabled":true}))
        .unwrap();
    assert_eq!(result["results"][0]["success"], true);
    assert_eq!(
        fs::read(_root.join("legacy.pak")).unwrap(),
        b"old-version-content"
    );
}

#[test]
fn deleting_requires_current_confirmation_and_never_forgets_live_files() {
    let (_t, mut s, _g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let m = &snap["mods"][0];
    assert!(s
        .request(json!({"action":"deleteMod","id":m["id"]}))
        .is_err());
    assert!(s
        .request(json!({"action":"deleteMod","id":m["id"],"confirmed":true}))
        .is_err());
    assert!(s
        .request(json!({"action":"forgetMod","id":m["id"],"confirmed":true}))
        .is_err());
    fs::write(r.join("a.utoc"), b"new member").unwrap();
    s.request(json!({"action":"scan"})).unwrap();
    assert!(s.request(json!({"action":"deleteMod","id":m["id"],"confirmed":true,"expectedMembers":m["members"]})).is_err());
    assert!(r.join("a.pak").exists());
    assert!(r.join("a.utoc").exists());
}

#[test]
fn missing_record_removal_does_not_delete_unrelated_files() {
    let (_t, mut s, _g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    fs::remove_file(r.join("a.pak")).unwrap();
    fs::write(r.join("other.pak"), b"other").unwrap();
    let snap = s
        .request(json!({"action":"forgetMod","id":id,"confirmed":true}))
        .unwrap();
    assert!(!snap["mods"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"] == id));
    assert_eq!(fs::read(r.join("other.pak")).unwrap(), b"other");
}

#[cfg(windows)]
#[test]
fn real_recycle_of_group_disabled_members_and_directory() {
    let (_t, mut s, _g, r) = setup();
    for ext in ["pak", "utoc", "ucas"] {
        fs::write(r.join(format!("outfit.{ext}")), b"disposable test data").unwrap();
    }
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let id = snap["mods"][0]["id"].clone();
    s.request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    let snap = s.request(json!({"action":"snapshot"})).unwrap();
    let m = &snap["mods"][0];
    let result = s
        .request(
            json!({"action":"deleteMod","id":id,"confirmed":true,"expectedMembers":m["members"]}),
        )
        .unwrap();
    assert!(result["mods"].as_array().unwrap().is_empty());
    assert_eq!(fs::read_dir(&r).unwrap().count(), 0);
    let folder = r.join("folder-mod");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("inner.bin"), b"disposable contents").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let m = &snap["mods"][0];
    s.request(
        json!({"action":"deleteMod","id":m["id"],"confirmed":true,"expectedMembers":m["members"]}),
    )
    .unwrap();
    assert!(!folder.exists());
    let count: i64 = s
        .conn
        .query_row(
            "SELECT COUNT(*) FROM deletions WHERE status='done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[cfg(windows)]
#[test]
fn recycle_locked_member_reports_failure_and_preserves_remaining_record() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_t, mut s, _g, r) = setup();
    fs::write(r.join("a.pak"), b"disposable a").unwrap();
    fs::write(r.join("a.utoc"), b"locked test member").unwrap();
    let snap = s.request(json!({"action":"scan"})).unwrap();
    let m = &snap["mods"][0];
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(r.join("a.utoc"))
        .unwrap();
    assert!(s.request(json!({"action":"deleteMod","id":m["id"],"confirmed":true,"expectedMembers":m["members"]})).is_err());
    assert!(r.join("a.utoc").exists());
    assert!(s
        .lib
        .mods
        .iter()
        .any(|item| item.id == m["id"].as_str().unwrap()));
    drop(lock);
}

#[test]
fn interrupted_delete_is_reconciled_without_retrying_file_deletion() {
    let (t, mut s, _g, r) = setup();
    fs::write(r.join("a.pak"), b"a").unwrap();
    fs::write(r.join("a.utoc"), b"b").unwrap();
    s.request(json!({"action":"scan"})).unwrap();
    let record = s.lib.mods[0].clone();
    s.conn
        .execute(
            "INSERT INTO deletions(id,data,status) VALUES(?1,?2,'pending')",
            params![uid(), serde_json::to_string(&record).unwrap()],
        )
        .unwrap();
    fs::remove_file(r.join("a.pak")).unwrap();
    let data = t.path().join("data");
    drop(s);
    let mut s = Service::new(data).unwrap();
    let snapshot = s.request(json!({"action":"snapshot"})).unwrap();
    assert!(r.join("a.utoc").exists());
    assert_eq!(snapshot["mods"][0]["state"], "missing");
    assert!(snapshot["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().contains("删除被中断")));
}
#[test]
fn recursive_suffix_directory_keeps_root_and_restores_every_file_after_restart() {
    let (t, mut s, g, r) = setup();
    let root = r.join("bundle");
    fs::create_dir_all(root.join("nested")).unwrap();
    for p in [
        "a.pak",
        "nested/b.dll",
        "nested/c.zip",
        "nested/config.json",
    ] {
        fs::write(root.join(p), p).unwrap();
    }
    let out = s.request(json!({"action":"scan"})).unwrap();
    let id = out["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"managed":true,"description":"folder note"}))
        .unwrap();
    let out = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(out["results"][0]["success"], true, "{out}");
    assert!(root.is_dir());
    for p in [
        "a.pak",
        "nested/b.dll",
        "nested/c.zip",
        "nested/config.json",
    ] {
        assert!(!root.join(p).exists());
        assert_eq!(
            fs::read_to_string(root.join(format!("{p}.disabled"))).unwrap(),
            p
        );
    }
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    let out = s.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(out["mods"][0]["state"], "disabled");
    let member = out["mods"][0]["members"][0]["id"].clone();
    s.request(json!({"action":"renameMember","id":id,"memberId":member,"name":"中文文件夹"}))
        .unwrap();
    assert!(r.join("中文文件夹/nested/c.zip.disabled").exists());
    assert!(!root.exists());
    let out = s
        .request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    assert_eq!(out["results"][0]["success"], true, "{out}");
    assert!(r.join("中文文件夹/nested/config.json").exists());
    let out = s.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(out["mods"][0]["description"], "folder note");
    assert_eq!(out["mods"][0]["id"], id);
    assert_eq!(out["mods"].as_array().unwrap().len(), 1);
    let b = t.path().join("tree-backup.json");
    s.request(json!({"action":"export","path":b})).unwrap();
    s.request(json!({"action":"import","path":b})).unwrap();
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["mods"][0]["state"],
        "enabled"
    );
    let _ = g;
}
#[test]
fn real_member_rename_preserves_group_note_id_extension_and_disabled_state() {
    let (t, mut s, _, r) = setup();
    for ext in ["pak", "utoc", "ucas"] {
        fs::write(r.join(format!("dress.{ext}")), ext).unwrap();
    }
    let out = s.request(json!({"action":"scan"})).unwrap();
    let id = out["mods"][0]["id"].clone();
    let member = out["mods"][0]["members"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"description":"keep note"}))
        .unwrap();
    s.request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    let ext = Path::new(
        out["mods"][0]["members"][0]["originalPath"]
            .as_str()
            .unwrap(),
    )
    .extension()
    .unwrap()
    .to_str()
    .unwrap();
    s.request(
        json!({"action":"renameMember","id":id,"memberId":member,"name":format!("礼服.{ext}")}),
    )
    .unwrap();
    assert!(r.join(format!("礼服.{ext}.disabled")).exists());
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    let out = s.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(out["mods"].as_array().unwrap().len(), 1);
    assert_eq!(out["mods"][0]["id"], id);
    assert_eq!(out["mods"][0]["description"], "keep note");
    assert_eq!(out["mods"][0]["members"].as_array().unwrap().len(), 3);
    let out = s
        .request(json!({"action":"toggle","ids":[id],"enabled":true}))
        .unwrap();
    assert_eq!(out["results"][0]["success"], true, "{out}");
    assert!(r.join(format!("礼服.{ext}")).exists());
}
#[test]
fn zip_rename_and_invalid_or_colliding_names_never_overwrite() {
    let (_t, mut s, _, r) = setup();
    fs::write(r.join("pack.zip"), "zip bytes").unwrap();
    let out = s.request(json!({"action":"scan"})).unwrap();
    let id = out["mods"][0]["id"].clone();
    let member = out["mods"][0]["members"][0]["id"].clone();
    fs::write(r.join("other.zip.disabled"), "other bytes").unwrap();
    for name in [
        "../bad.zip",
        "CON.zip",
        "other.zip",
        "pack.pak",
        "pack.zip.disabled",
        "PACK.zip",
    ] {
        assert!(s
            .request(json!({"action":"renameMember","id":id,"memberId":member,"name":name}))
            .is_err());
    }
    assert_eq!(fs::read_to_string(r.join("pack.zip")).unwrap(), "zip bytes");
    assert_eq!(
        fs::read_to_string(r.join("other.zip.disabled")).unwrap(),
        "other bytes"
    );
    s.request(json!({"action":"editMod","id":id,"managed":true}))
        .unwrap();
    s.request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    s.request(json!({"action":"renameMember","id":id,"memberId":member,"name":"材质包.zip"}))
        .unwrap();
    assert_eq!(
        fs::read_to_string(r.join("材质包.zip.disabled")).unwrap(),
        "zip bytes"
    );
}
#[test]
fn directory_conflict_preflight_and_new_files_report_real_mixed_state() {
    let (_t, mut s, _, r) = setup();
    let root = r.join("bundle");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("a.txt"), "a").unwrap();
    fs::write(root.join("z.txt"), "z").unwrap();
    let out = s.request(json!({"action":"scan"})).unwrap();
    let id = out["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"managed":true}))
        .unwrap();
    fs::write(root.join("z.txt.disabled"), "collision").unwrap();
    let out = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(out["results"][0]["success"], false);
    assert!(root.join("a.txt").exists());
    assert_eq!(
        fs::read_to_string(root.join("z.txt.disabled")).unwrap(),
        "collision"
    );
    fs::remove_file(root.join("z.txt.disabled")).unwrap();
    let out = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(out["results"][0]["success"], true, "{out}");
    fs::write(root.join("new.pak"), "new").unwrap();
    let out = s.request(json!({"action":"scan"})).unwrap();
    assert_eq!(out["mods"][0]["state"], "mixed");
    let out = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(out["results"][0]["success"], true, "{out}");
    assert!(root.join("new.pak.disabled").exists());
    assert!(!root.join("a.txt.disabled.disabled").exists());
}
#[test]
fn tree_manifest_backup_cannot_authorize_outside_files() {
    let (t, mut s, _, r) = setup();
    fs::create_dir(r.join("bundle")).unwrap();
    fs::write(r.join("bundle/a.txt"), "a").unwrap();
    s.request(json!({"action":"scan"})).unwrap();
    let b = t.path().join("b.json");
    s.request(json!({"action":"export","path":b})).unwrap();
    let mut raw: Value = serde_json::from_slice(&fs::read(&b).unwrap()).unwrap();
    raw["library"]["mods"][0]["members"][0]["leaves"][0]["original"] =
        json!(t.path().join("outside.txt"));
    fs::write(&b, serde_json::to_vec(&raw).unwrap()).unwrap();
    assert!(s.request(json!({"action":"import","path":b})).is_err());
    assert!(r.join("bundle/a.txt").exists());
}
#[test]
fn interrupted_real_rename_rolls_back_and_restores_metadata() {
    let (t, mut s, _, r) = setup();
    fs::write(r.join("a.pak"), "bytes").unwrap();
    let out = s.request(json!({"action":"scan"})).unwrap();
    let before = s.lib.clone();
    let identity = disk_identity(&r.join("a.pak")).unwrap();
    let record = json!({"before":before,"steps":[{"from":r.join("a.pak"),"to":r.join("b.pak")}],"identities":[identity]});
    s.conn
        .execute(
            "INSERT INTO file_edits(id,data) VALUES('crash',?1)",
            params![record.to_string()],
        )
        .unwrap();
    no_replace_move(&r.join("a.pak"), &r.join("b.pak")).unwrap();
    s.lib.mods[0].members[0].original_path = r.join("b.pak");
    s.lib.mods[0].members[0].current_path = r.join("b.pak");
    s.persist().unwrap();
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    assert!(r.join("a.pak").exists());
    assert!(!r.join("b.pak").exists());
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["mods"][0]["id"],
        out["mods"][0]["id"]
    );
    s.ensure_no_journals().unwrap();
}
#[cfg(windows)]
#[test]
fn tree_locked_member_rolls_back_other_members() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_t, mut s, _, r) = setup();
    let root = r.join("bundle");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("a.txt"), "a").unwrap();
    fs::write(root.join("z.txt"), "z").unwrap();
    let out = s.request(json!({"action":"scan"})).unwrap();
    let id = out["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"managed":true}))
        .unwrap();
    s.lib.mods[0].members[0]
        .leaves
        .sort_by_key(|l| norm(&l.original));
    s.persist().unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(root.join("z.txt"))
        .unwrap();
    let out = s
        .request(json!({"action":"toggle","ids":[id],"enabled":false}))
        .unwrap();
    assert_eq!(out["results"][0]["success"], false);
    assert!(root.join("a.txt").exists());
    assert!(!root.join("a.txt.disabled").exists());
    assert!(root.join("z.txt").exists());
    drop(lock);
    s.ensure_no_journals().unwrap();
}
#[test]
fn replaced_tree_leaf_requires_explicit_review_before_toggle() {
    let (_t, mut s, _, r) = setup();
    fs::create_dir(r.join("bundle")).unwrap();
    fs::write(r.join("bundle/a.txt"), "a").unwrap();
    let out = s.request(json!({"action":"scan"})).unwrap();
    let id = out["mods"][0]["id"].clone();
    s.request(json!({"action":"editMod","id":id,"managed":true,"description":"notes"}))
        .unwrap();
    no_replace_move(&r.join("bundle/a.txt"), &r.join("previous.txt")).unwrap();
    fs::write(r.join("bundle/a.txt"), "replacement").unwrap();
    let out = s.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(out["mods"][0]["state"], "error");
    assert_eq!(out["mods"][0]["identityChanged"], true);
    assert_eq!(
        s.request(json!({"action":"toggle","ids":[id],"enabled":false}))
            .unwrap()["results"][0]["success"],
        false
    );
    s.request(json!({"action":"acceptReplacement","id":id,"confirmed":true}))
        .unwrap();
    assert_eq!(
        s.request(json!({"action":"toggle","ids":[id],"enabled":false}))
            .unwrap()["results"][0]["success"],
        true
    );
    assert_eq!(
        fs::read_to_string(r.join("bundle/a.txt.disabled")).unwrap(),
        "replacement"
    );
}
#[test]
fn unresolved_edit_reports_error_blocks_operations_then_recovers_after_conflict_resolved() {
    let (t, mut s, _, r) = setup();
    fs::write(r.join("a.pak"), "a").unwrap();
    let out = s.request(json!({"action":"scan"})).unwrap();
    let id = out["mods"][0]["id"].clone();
    let record = json!({"before":s.lib,"steps":[{"from":r.join("a.pak"),"to":r.join("b.pak")}],"identities":[disk_identity(&r.join("a.pak"))]});
    s.conn
        .execute(
            "INSERT INTO file_edits(id,data) VALUES('crash',?1)",
            params![record.to_string()],
        )
        .unwrap();
    no_replace_move(&r.join("a.pak"), &r.join("b.pak")).unwrap();
    fs::write(r.join("a.pak"), "conflicting unrelated file").unwrap();
    drop(s);
    let mut s = Service::new(t.path().join("data")).unwrap();
    let out = s.request(json!({"action":"snapshot"})).unwrap();
    assert_eq!(out["mods"].as_array().unwrap().len(), 1);
    assert_eq!(out["mods"][0]["state"], "error");
    assert!(s.ensure_no_journals().is_err());
    assert_eq!(
        fs::read_to_string(r.join("a.pak")).unwrap(),
        "conflicting unrelated file"
    );
    drop(s);
    fs::remove_file(r.join("a.pak")).unwrap();
    let mut s = Service::new(t.path().join("data")).unwrap();
    assert_eq!(fs::read_to_string(r.join("a.pak")).unwrap(), "a");
    assert_eq!(
        s.request(json!({"action":"snapshot"})).unwrap()["mods"][0]["id"],
        id
    );
    s.ensure_no_journals().unwrap();
}
