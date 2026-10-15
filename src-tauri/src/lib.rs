pub mod assets;
mod clipboard_image;
pub mod core;
mod monitoring;

use fs2::FileExt;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{Emitter, Manager};

pub struct AppState {
    startup_ready: Arc<AtomicBool>,
    service: Arc<Mutex<core::Service>>,
    monitor: Arc<Mutex<monitoring::MonitorStatus>>,
    imports: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    _instance_lock: File,
}

#[tauri::command]
fn sync_theme(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    theme: String,
    dark: bool,
    ready: bool,
) -> Result<(), String> {
    let native = match theme.as_str() {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        "system" => None,
        _ => return Err("外观设置无效".into()),
    };
    window.set_theme(native).map_err(|e| e.to_string())?;
    let color = if dark {
        tauri::window::Color(23, 29, 40, 255)
    } else {
        tauri::window::Color(252, 253, 255, 255)
    };
    window
        .set_background_color(Some(color))
        .map_err(|e| e.to_string())?;
    if ready && !state.startup_ready.load(Ordering::Acquire) {
        window.show().map_err(|e| e.to_string())?;
        state.startup_ready.store(true, Ordering::Release);
    }
    Ok(())
}

struct ImportRegistration {
    id: String,
    imports: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}
impl Drop for ImportRegistration {
    fn drop(&mut self) {
        if let Ok(mut imports) = self.imports.lock() {
            imports.remove(&self.id);
        }
    }
}

#[tauri::command]
async fn request(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    mut request: Value,
) -> Result<Value, String> {
    let action = request
        .get("action")
        .and_then(Value::as_str)
        .ok_or("请求缺少操作类型")?;
    if action == "clipboardImage" {
        return tauri::async_runtime::spawn_blocking(clipboard_image::read)
            .await
            .map_err(|_| "剪贴板读取失败，请重试")?;
    }
    // Cancellation never waits for the service mutex held by the active import.
    if action == "cancelImport" {
        let id = request
            .get("operationId")
            .and_then(Value::as_str)
            .ok_or("缺少导入操作编号")?;
        let imports = state
            .imports
            .lock()
            .map_err(|_| "导入状态异常，请重新打开程序")?;
        let token = imports.get(id);
        if let Some(token) = token {
            token.store(true, Ordering::Relaxed);
        }
        return Ok(json!({"operationId":id,"cancelRequested":token.is_some()}));
    }
    let controlled_import = if action == "importFiles" {
        let id = match request.get("operationId").and_then(Value::as_str) {
            Some(id) => uuid::Uuid::parse_str(id)
                .map_err(|_| "导入操作编号格式错误")?
                .to_string(),
            None => uuid::Uuid::new_v4().to_string(),
        };
        let token = Arc::new(AtomicBool::new(false));
        let mut imports = state
            .imports
            .lock()
            .map_err(|_| "导入状态异常，请重新打开程序")?;
        if imports.contains_key(&id) {
            return Err("该导入操作正在执行".into());
        }
        imports.insert(id.clone(), token.clone());
        request["operationId"] = json!(id);
        Some((
            token,
            ImportRegistration {
                id,
                imports: state.imports.clone(),
            },
        ))
    } else {
        None
    };
    let service = state.service.clone();
    let monitor = state.monitor.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let controlled_import = controlled_import;
        let mut service = service
            .lock()
            .map_err(|_| "管理服务状态异常，请重新打开程序".to_string())?;
        let action = request
            .get("action")
            .and_then(Value::as_str)
            .ok_or("请求缺少操作类型")?;
        if let Some((token, _registration)) = controlled_import.as_ref() {
            let mut progress = |value: Value| {
                let _ = app.emit("import-progress", value);
            };
            let result = service.import_files_controlled(&request, token, &mut progress);
            let _ = app.emit("library-changed", ());
            return result;
        }
        if action == "revealMember" {
            let snapshot = service.request(json!({"action":"snapshot"}))?;
            let id = request["id"].as_str().ok_or("缺少 MOD")?;
            let member = request["memberId"].as_str().ok_or("缺少成员")?;
            let path = snapshot["mods"]
                .as_array()
                .and_then(|mods| mods.iter().find(|m| m["id"].as_str() == Some(id)))
                .and_then(|m| m["members"].as_array())
                .and_then(|members| members.iter().find(|m| m["id"].as_str() == Some(member)))
                .and_then(|m| m["currentPath"].as_str())
                .ok_or("成员不存在，请刷新后重试")?;
            let path = PathBuf::from(path);
            let ordinary = path.to_string_lossy();
            let ordinary = ordinary.strip_prefix("\\\\?\\").unwrap_or(&ordinary);
            if path.exists() {
                std::process::Command::new("explorer.exe")
                    .arg(format!("/select,\"{ordinary}\""))
                    .spawn()
                    .map_err(|e| format!("无法定位文件：{e}"))?;
            } else {
                let parent = path
                    .parent()
                    .filter(|p| p.is_dir())
                    .ok_or("所在目录已经不存在")?;
                std::process::Command::new("explorer.exe")
                    .arg(parent)
                    .spawn()
                    .map_err(|e| format!("无法打开文件夹：{e}"))?;
            }
            return Ok(json!({"success":true}));
        }
        if action == "openRecycleBin" {
            std::process::Command::new("explorer.exe")
                .arg("shell:RecycleBinFolder")
                .spawn()
                .map_err(|e| format!("无法打开文件夹：{e}"))?;
            return Ok(json!({"success":true}));
        }
        if action == "openFolder" {
            let path = service.folder_path(
                request.get("gameId").and_then(Value::as_str),
                request.get("folderId").and_then(Value::as_str),
            )?;
            drop(service);
            std::process::Command::new("explorer.exe")
                .arg(path)
                .spawn()
                .map_err(|e| format!("无法打开目录：{e}"))?;
            return Ok(json!({"success":true}));
        }
        let mut result = service.request_with_progress(request.clone(), |progress| {
            let _ = app.emit("toggle-progress", progress);
        })?;
        if result.get("games").is_some()
            || result.get("mods").is_some()
            || result["unchanged"] == true
        {
            if let Ok(status) = monitor.lock() {
                result["monitoring"] = json!(status.label);
                if let Some(warnings) = result["warnings"].as_array_mut() {
                    warnings.extend(status.warnings.iter().map(|w| json!(w)));
                }
            }
        }
        if action != "snapshot"
            && action != "viewSnapshot"
            && action != "toggle"
            && action != "export"
            && action != "previewImage"
            && action != "saveCrop"
            && action != "saveView"
            && request.get("quiet").and_then(Value::as_bool) != Some(true)
            && !(action == "settings"
                && request.get("compact").and_then(Value::as_bool) == Some(true))
        {
            let _ = app.emit("library-changed", ());
        }
        Ok(result)
    })
    .await
    .map_err(|e| format!("后台任务未完成：{e}"))?
}

pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            #[cfg(debug_assertions)]
            let data_dir = std::env::var_os("MM_TEST_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(data_dir);
            std::fs::create_dir_all(&data_dir)?;
            app.asset_protocol_scope()
                .allow_directory(data_dir.join("images"), true)?;
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(data_dir.join("instance.lock"))?;
            lock.try_lock_exclusive()
                .map_err(|_| "MOD Manager 已经运行，请返回已打开的窗口")?;
            let service = core::Service::new(data_dir.clone()).map_err(std::io::Error::other)?;
            let preference=service.theme_preference().to_string();
            let service = Arc::new(Mutex::new(service));
            let startup_ready=Arc::new(AtomicBool::new(false));
            let monitor = Arc::new(Mutex::new(monitoring::MonitorStatus::default()));
            monitoring::start(app.handle().clone(), service.clone(), monitor.clone());
            app.manage(AppState {
                startup_ready:startup_ready.clone(),
                service,
                monitor,
                imports: Arc::new(Mutex::new(HashMap::new())),
                _instance_lock: lock,
            });
            let native_theme=match preference.as_str(){"dark"=>Some(tauri::Theme::Dark),"light"=>Some(tauri::Theme::Light),_=>None};
            let background=if preference=="dark" {tauri::window::Color(23,29,40,255)} else {tauri::window::Color(252,253,255,255)};
            let initialization=include_str!("startup-theme.js").replace("__THEME_PREFERENCE__",&serde_json::to_string(&preference)?);
            let window = tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .visible(false).theme(native_theme).background_color(background).initialization_script(initialization);
            #[cfg(debug_assertions)]
            let window = if std::env::var_os("MM_TEST_DATA_DIR").is_some() {
                window
                    .data_directory(data_dir.join("webview"))
                    .additional_browser_args("--remote-debugging-port=9222")
            } else {
                window
            };
            window.build()?;
            // A failed page/module must not leave an invisible, locked instance.
            let handle=app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(30));
                if !startup_ready.load(Ordering::Acquire) {
                    let exit_handle=handle.clone();
                    let _=handle.run_on_main_thread(move || {
                        if !startup_ready.load(Ordering::Acquire) {
                            show_startup_error("界面加载未完成，请关闭程序后重试，并检查 WebView2 运行环境。\n管理器没有执行新的 MOD 启停操作。");
                            exit_handle.exit(1);
                        }
                    });
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![request,sync_theme])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        show_startup_error(&format!(
            "无法启动 MOD Manager。请检查程序是否已运行、WebView2 和数据目录权限。\n\n{error}"
        ));
    }
}

fn show_startup_error(message: &str) {
    eprintln!("{message}");
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(
                window: *mut std::ffi::c_void,
                text: *const u16,
                caption: *const u16,
                kind: u32,
            ) -> i32;
        }
        let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let caption: Vec<u16> = "Mod Manager".encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10);
        }
    }
}
