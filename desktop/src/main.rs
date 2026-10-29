mod backend;
mod clipboard;
mod restart;
mod service;
use serde_json::Value;
use service::{DashboardService, LocalHttpService, Reply};
use std::{
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

struct NativeState {
    service: Arc<LocalHttpService>,
    verified: Arc<AtomicBool>,
    connections: Arc<Mutex<Option<(PathBuf, Arc<backend::connections::SetupHost>)>>>,
}

fn active_profile(service:&LocalHttpService)->Result<PathBuf,String>{
    service.health()?;
    let reply = service.request("/api/commands/invoke", Some(serde_json::json!({"name":"diagnostics.storage"})))?;
    if reply.status != 200 { return Err("Could not identify the active service profile.".into()); }
    let state_path = reply.data["statePath"].as_str().ok_or("The service did not identify its profile.")?;
    PathBuf::from(state_path).parent().ok_or("Invalid service profile path.")?.canonicalize().map_err(|_|"The active service profile is unavailable.".into())
}
fn connection_host(service: &LocalHttpService, cached: &Mutex<Option<(PathBuf, Arc<backend::connections::SetupHost>)>>) -> Result<Arc<backend::connections::SetupHost>, String> {
    let profile=active_profile(service)?;
    let mut cached = cached.lock().map_err(|_|"Connection setup is unavailable.")?;
    if let Some((path, host)) = cached.as_ref() { if path == &profile { return Ok(host.clone()); } host.cancel(); }
    let host = Arc::new(backend::connections::SetupHost::new(profile.clone()).map_err(|error|error.message)?);
    *cached = Some((profile,host.clone()));
    Ok(host)
}
#[tauri::command]
async fn connection_setup_view(state: State<'_,NativeState>) -> Result<Value,String> {
    let service=state.service.clone();let cached=state.connections.clone();
    tauri::async_runtime::spawn_blocking(move||connection_host(&service,&cached)?.view().map_err(|error|error.message)).await.map_err(|_|"Connection setup failed.".to_string())?
}
#[tauri::command]
async fn connection_setup_cancel(state: State<'_,NativeState>) -> Result<Value,String> {
    let cached=state.connections.clone();
    tauri::async_runtime::spawn_blocking(move||{
        let cached=cached.lock().map_err(|_|"Connection setup is unavailable.".to_string())?;
        Ok(cached.as_ref().map(|(_,host)|host.cancel()).unwrap_or_else(||serde_json::json!({"cancelled":true})))
    }).await.map_err(|_|"Could not cancel connection setup.".to_string())?
}
#[tauri::command]
async fn connection_setup_preview(app:tauri::AppHandle,state:State<'_,NativeState>,payload:Value)->Result<Value,String>{
    let service=state.service.clone();let cached=state.connections.clone();
    tauri::async_runtime::spawn_blocking(move||{
        let host=connection_host(&service,&cached)?;
        let ticket=host.begin_preview();
        if payload["source"]=="github-cli" {
            let approved=app.dialog().message("Read your GitHub CLI token to validate its account and repository access? The token stays outside chat and is saved only after a separate approval.").title("Use GitHub CLI login").buttons(MessageDialogButtons::OkCancelCustom("Read token".into(),"Cancel".into())).blocking_show();
            if !approved { host.cancel();return Err("GitHub CLI access cancelled.".into()); }
        }
        let current=connection_host(&service,&cached)?;
        if !Arc::ptr_eq(&host,&current){host.cancel();return Err("The active service profile changed. Validate this connection again.".into());}
        host.preview_with_ticket(&payload,ticket).map_err(|error|error.message)
    }).await.map_err(|_|"Connection validation failed.".to_string())?
}
#[tauri::command]
async fn connection_setup_commit(app:tauri::AppHandle,state:State<'_,NativeState>,payload:Value)->Result<Value,String>{
    let service=state.service.clone();let cached=state.connections.clone();
    tauri::async_runtime::spawn_blocking(move||{
        let host=connection_host(&service,&cached)?;
        let confirmation=host.confirmation(&payload).map_err(|error|error.message)?;
        let message=format!("Save the {} connection for {} in macOS Keychain?\n\nActive profile: {}\n\nThis replaces this profile's saved connection for this provider. Existing synchronization settings still apply.",backend::text(&confirmation,"provider"),backend::text(&confirmation,"identity"),backend::text(&confirmation,"profile"));
        let approved=app.dialog().message(message).title("Save connection").buttons(MessageDialogButtons::OkCancelCustom("Save connection".into(),"Cancel".into())).blocking_show();
        if !approved {host.cancel();return Err("Connection save cancelled.".into());}
        let current=connection_host(&service,&cached)?;
        if !Arc::ptr_eq(&host,&current){host.cancel();return Err("The active service profile changed. Validate this connection again.".into());}
        host.commit(&payload).map_err(|error|error.message)
    }).await.map_err(|_|"Connection save failed.".to_string())?
}

#[tauri::command]
async fn service_request(
    state: State<'_, NativeState>,
    path: String,
    payload: Option<Value>,
) -> Result<Reply, String> {
    let service = state.service.clone();
    let verified = state.verified.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !verified.load(Ordering::Relaxed) {
            service.health()?;
            verified.store(true, Ordering::Relaxed);
        }
        service.request(&path, payload)
    })
    .await
    .map_err(|_| "Service request failed.".to_owned())
    .map(|result| result.unwrap_or_else(service::error_reply))
}
#[tauri::command]
async fn read_artifact(state: State<'_, NativeState>, id: String) -> Result<String, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let asset = service.artifact(&id)?;
        if !asset.content_type.starts_with("text/plain") && !asset.content_type.starts_with("text/html") {
            return Err("This artifact is not text.".into());
        }
        String::from_utf8(asset.bytes).map_err(|_| "The artifact is not UTF-8 text.".into())
    })
    .await
    .map_err(|_| "Artifact read failed.".to_owned())?
}
#[tauri::command]
async fn copy_text(app: tauri::AppHandle, text: String) -> Result<(),String> {
    if text.len()>32*1024*1024{return Err("Clipboard text exceeds 32 MiB.".into());}
    tauri::async_runtime::spawn_blocking(move||{
        let(tx,rx)=std::sync::mpsc::channel();
        app.run_on_main_thread(move||{let _=tx.send(clipboard::text(&text));}).map_err(|_|"Could not access the clipboard.".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|_|"Clipboard write timed out.".to_string())?
    }).await.map_err(|_|"Could not copy text.".to_string())?
}
#[tauri::command]
async fn copy_artifact_image(app:tauri::AppHandle,state: State<'_,NativeState>,id:String)->Result<(),String>{
    let service=state.service.clone();
    tauri::async_runtime::spawn_blocking(move||{
        let asset=service.artifact(&id)?;
        if !["image/png","image/jpeg","image/webp"].contains(&asset.content_type.split(';').next().unwrap_or("")){return Err("Choose a PNG, JPEG, or WebP artifact.".into());}
        let(tx,rx)=std::sync::mpsc::channel();
        app.run_on_main_thread(move||{let _=tx.send(clipboard::image(&asset.bytes));}).map_err(|_|"Could not access the clipboard.".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|_|"Image clipboard write timed out.".to_string())?
    }).await.map_err(|_|"Could not copy the image.".to_string())?
}
#[tauri::command]
async fn restart_dashboard_preview(state:State<'_,NativeState>)->Result<Value,String>{
    let url=state.service.origin();tauri::async_runtime::spawn_blocking(move||restart::preview(&restart::root(),&url)).await.map_err(|_|"Could not inspect restart state.".to_string())?
}
#[tauri::command]
async fn restart_dashboard_status()->Result<Value,String>{
    tauri::async_runtime::spawn_blocking(move||restart::status(&restart::root())).await.map_err(|_|"Could not read restart state.".to_string())?
}
#[tauri::command]
async fn restart_dashboard_start(state:State<'_,NativeState>,mode:String,confirmed:bool,snapshot:String)->Result<Value,String>{
    let url=state.service.origin();tauri::async_runtime::spawn_blocking(move||restart::start(&restart::root(),&url,&mode,confirmed,&snapshot)).await.map_err(|_|"Could not start restart helper.".to_string())?
}
#[tauri::command]
async fn service_status(state: State<'_, NativeState>) -> Result<Value, String> {
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.health())
        .await
        .map_err(|_| "Health check failed.".to_owned())?
}
#[tauri::command]
fn open_cmux() -> Result<(), String> {
    Command::new("/usr/bin/open")
        .args(["-a", "cmux"])
        .spawn()
        .map(|_| ())
        .map_err(|_| "Could not open cmux.".into())
}
fn zed_directory(path: &str) -> Result<PathBuf, String> {
    let directory = PathBuf::from(path);
    if !directory.is_absolute() {
        return Err("Choose an absolute working directory.".into());
    }
    let directory = directory.canonicalize()
        .map_err(|_| "The agent's working directory no longer exists.".to_owned())?;
    if !directory.is_dir() {
        return Err("The agent's working directory is not a folder.".into());
    }
    Ok(directory)
}
#[tauri::command]
async fn open_git_diff_in_zed(state:State<'_,NativeState>,path: String) -> Result<(), String> {
    let service=state.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let profile=active_profile(&service)?;
        let directory = zed_directory(&path)?;
        let prefs=backend::settings::Store::load(profile.join("settings.json")).map_err(|e|e.message)?.values();
        let editor=backend::text(&prefs,"editor");
        let args=if editor=="zed"{backend::git_diff::zed_arguments(&directory,&profile.join("git-diffs")).map_err(|e|e.message)?}else{vec![directory.to_string_lossy().into_owned()]};
        backend::terminal::run(&backend::terminal::executable(editor), &args, None, 15)
            .map_err(|error| error.message)?;
        Ok(())
    }).await.map_err(|_| "Could not open the Git diff in Zed.".to_owned())?
}
#[tauri::command]
async fn open_settings_file(state:State<'_,NativeState>)->Result<(),String>{
    let service=state.service.clone();
    tauri::async_runtime::spawn_blocking(move||{
        let profile=active_profile(&service)?;
        let path=profile.join("settings.json");
        let store=backend::settings::Store::load(path.clone()).map_err(|error|error.message)?;
        let values=store.values();let editor=backend::text(&values,"editor");
        Command::new(backend::terminal::executable(editor)).arg(path).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map_err(|_|format!("Could not open settings in {editor}. Check that its CLI is installed."))?;
        Ok(())
    }).await.map_err(|_|"Could not open the settings file.".to_string())?
}
#[tauri::command]
fn set_shortcut_recording(app:tauri::AppHandle,recording:bool)->Result<(),String>{
    if let Some(menu)=app.menu(){for item in menu.items().map_err(|e|e.to_string())?{if let Some(submenu)=item.as_submenu(){submenu.set_enabled(!recording).map_err(|e|e.to_string())?;}}}
    Ok(())
}
#[tauri::command]
fn apply_native_settings(app:tauri::AppHandle, values:Value, light:bool)->Result<(),String>{
    backend::settings::validate(&values).map_err(|e|e.message)?;
    if let Some(menu)=app.menu(){
        let ids=["toggle-left","toggle-right","commands","find-work","page-search","settings","agent-new","agent-close","agent-reopen","agent-rename","chat-terminal"];
        for id in ids{if let Some(item)=menu.get(id){if let Some(item)=item.as_menuitem(){item.set_accelerator(None::<&str>).map_err(|e|e.to_string())?;}}}
        for id in ids{if let Some(item)=menu.get(id){if let Some(item)=item.as_menuitem(){let chord=values["keybindings"][id].as_str().unwrap_or("").replace("Primary","CmdOrCtrl");if !chord.is_empty(){item.set_accelerator(Some(chord)).map_err(|e|e.to_string())?;}}}}
    }
    if let Some(window)=app.get_webview_window("main"){window.set_zoom(values["uiScale"].as_f64().unwrap_or(100.0)/100.0).map_err(|e|e.to_string())?;window.set_theme(Some(if light{tauri::Theme::Light}else{tauri::Theme::Dark})).map_err(|e|e.to_string())?;}
    Ok(())
}
#[tauri::command]
fn drag_titlebar(window: tauri::WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|_| "Could not move the window.".into())
}
#[tauri::command]
fn desktop_info(state: State<'_, NativeState>) -> Value {
    serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"profile":if cfg!(debug_assertions){"debug"}else{"release"},"builtAt":env!("MYDASH_BUILD_TIME"),"service":state.service.origin()})
}
fn external_url(url: &tauri::Url, service: &LocalHttpService) -> Result<String, String> {
    match url.scheme() {
        "https" | "http" if url.username().is_empty() && url.password().is_none() => {
            Ok(url.to_string())
        }
        "mailto" => Ok(url.to_string()),
        "mydash-artifact" if url.host_str() == Some("localhost") => {
            service.artifact_url(url.path().trim_start_matches('/'))
        }
        "tauri" if url.host_str() == Some("localhost") && url.path().starts_with("/linear/") => {
            service.linear_url(url.path())
        }
        _ => Err("Unsupported external link.".into()),
    }
}
fn open_external(url: &tauri::Url, service: &LocalHttpService) {
    if let Ok(value) = external_url(url, service) {
        let _ = Command::new("/usr/bin/open").arg("--").arg(value).spawn();
    }
}
fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if let Some(result)=backend::agent_cli::run(&args){if let Err(error)=result{eprintln!("{}",error.message);std::process::exit(1);}return;}
    if let Some(index)=args.iter().position(|x|x=="--restart-control") {
        let value=|key:&str|args.iter().position(|x|x==key).and_then(|i|args.get(i+1));
        let root=value("--home").map(PathBuf::from).unwrap_or_else(restart::root);
        let url=value("--url").map(String::as_str).unwrap_or("http://127.0.0.1:8765");
        let result=match args.get(index+1).map(String::as_str){
            Some("preview")=>restart::preview(&root,url),
            Some("status")=>restart::status(&root),
            Some("start")=>restart::start(&root,url,value("--mode").map(String::as_str).unwrap_or("service"),args.iter().any(|x|x=="--confirm-interruption"),value("--snapshot").map(String::as_str).unwrap_or("")),
            _=>Err("Use restart-control preview, status, or start.".into()),
        };
        match result{Ok(value)=>println!("{value}"),Err(error)=>{eprintln!("{error}");std::process::exit(1);}}return;
    }
    if args.iter().any(|x|x=="--restart-dashboard") {
        let value=|key:&str|args.iter().position(|x|x==key).and_then(|i|args.get(i+1));
        let result=match(value("--home"),value("--job")){
            (Some(root),Some(id))=>restart::run(&PathBuf::from(root),id),
            _=>Err("Restart helper requires --home and --job.".into()),
        };
        if let Err(error)=result{eprintln!("{error}");std::process::exit(1);}return;
    }
    if let Some(index) = args.iter().position(|x| x == "--check-agent-storage") {
        let agent = args.get(index + 1).map(String::as_str).unwrap_or("claude");
        match backend::embedded::check_agent_storage(agent) {
            Ok(()) => println!("Agent storage is writable."),
            Err(error) => {
                eprintln!("{}", error.message);
                std::process::exit(1);
            }
        }
        return;
    }
    if args.iter().any(|x| x == "--terminal-daemon") {
        let value = |name: &str| args.iter().position(|x| x == name).and_then(|i| args.get(i+1)).map(PathBuf::from);
        let result = match (value("--home"), value("--data")) {
            (Some(root), Some(data)) => backend::terminal_daemon::serve(root, data, args.iter().any(|x|x=="--allow-demo-terminals")),
            _ => Err(backend::Error::new("Terminal daemon requires --home and --data.")),
        };
        if let Err(error) = result {eprintln!("{}",error.message);std::process::exit(1);}
        return;
    }
    if args.iter().any(|x| x == "--serve") {
        let value = |name: &str| {
            args.iter()
                .position(|x| x == name)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        let root = value("--home")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("MYDASH_HOME").map(PathBuf::from))
            .unwrap_or_else(|| backend::home().join("code/mydash"));
        let port = value("--port").and_then(|x| x.parse().ok()).unwrap_or(8765);
        if let Err(e) = backend::http::serve(
            root,
            value("--data").map(PathBuf::from),
            port,
            args.iter().any(|x| x == "--demo"),
            value("--terminal-demo"),
        ) {
            eprintln!("{}", e.message);
            std::process::exit(1);
        }
        return;
    }

    let address = std::env::var("MYDASH_SERVICE_URL")
        .ok()
        .or_else(|| option_env!("MYDASH_SERVICE_URL").map(str::to_owned))
        .unwrap_or_else(|| "http://127.0.0.1:8765".into());
    let service =
        Arc::new(LocalHttpService::new(&address).expect("Invalid local service configuration"));
    let assets = service.clone();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(NativeState {
            service: service.clone(),
            verified: Arc::new(AtomicBool::new(false)),
            connections: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            service_request,
            service_status,
            connection_setup_view,
            connection_setup_preview,
            connection_setup_commit,
            connection_setup_cancel,
            restart_dashboard_preview,
            restart_dashboard_start,
            restart_dashboard_status,
            read_artifact,
            copy_text,
            copy_artifact_image,
            desktop_info,
            open_cmux,
            open_git_diff_in_zed,
            drag_titlebar,
            apply_native_settings,
            set_shortcut_recording,
            open_settings_file
        ])
        .register_asynchronous_uri_scheme_protocol(
            "mydash-artifact",
            move |_, request, responder| {
                let service = assets.clone();
                std::thread::spawn(move || {
                    let result = if request.method() == "GET" && request.uri().query().is_none() {
                        service.artifact(request.uri().path().trim_start_matches('/'))
                    } else {
                        Err("Invalid artifact request.".into())
                    };
                    let response = match result {
                        Ok(asset) => tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", asset.content_type)
                            .header("Content-Security-Policy", service::ARTIFACT_CSP)
                            .header("X-Content-Type-Options", "nosniff")
                            .header("Referrer-Policy", "no-referrer")
                            .header("Cache-Control", "no-store")
                            .body(asset.bytes),
                        Err(_) => tauri::http::Response::builder()
                            .status(404)
                            .header("Content-Type", "text/plain")
                            .body(b"Artifact unavailable.".to_vec()),
                    };
                    responder.respond(response.expect("Valid artifact response"));
                });
            },
        )
        .setup(move |app| {
            let navigation = service.clone();
            let new_windows = service.clone();
            let window_builder = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::App(PathBuf::from("index.html")),
            )
            .title("truffle")
            .inner_size(1360.0, 860.0)
            .min_inner_size(800.0, 540.0)
            ;
            #[cfg(target_os = "macos")]
            let window_builder = window_builder
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
                // Wry's y inset adjusts the native container height, not the button top.
                // 23 centers the controls in workspace.css's 40-point title bar.
                .traffic_light_position(tauri::LogicalPosition::new(12.0, 19.0));
            let window = window_builder
            .theme(Some(tauri::Theme::Dark))
            .background_color(tauri::window::Color(25, 23, 36, 255))
            .on_navigation(move |url| {
                if (url.scheme() == "tauri"
                    && url.host_str() == Some("localhost")
                    && !url.path().starts_with("/linear/"))
                    || (url.scheme() == "mydash-artifact" && url.host_str() == Some("localhost"))
                    || url.as_str() == "about:blank"
                {
                    return true;
                }
                open_external(url, &navigation);
                false
            })
            .on_new_window(move |url, _| {
                open_external(&url, &new_windows);
                tauri::webview::NewWindowResponse::Deny
            })
            .build()?;
            window.on_window_event(|event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                }
            });
            // Native menu provides About, Quit, Hide, and standard macOS Edit shortcuts.
            let menu = tauri::menu::Menu::default(app.handle())?;
            let hide_label = tauri::menu::PredefinedMenuItem::hide(app.handle(), None)?.text()?;
            let close_label = tauri::menu::PredefinedMenuItem::close_window(app.handle(), None)?.text()?;
            for item in menu.items()? {
                if let Some(submenu) = item.as_submenu() {
                    for child in submenu.items()? {
                        if let tauri::menu::MenuItemKind::Predefined(predefined) = &child {
                            if predefined.text()? == close_label || predefined.text()? == hide_label { submenu.remove(predefined)?; }
                        }
                    }
                    if submenu.text()? == "View" {
                        for (id,label,key) in [("toggle-left","Toggle Left Sidebar","CmdOrCtrl+H"),("toggle-right","Toggle Right Sidebar","CmdOrCtrl+L"),("commands","Commands…","CmdOrCtrl+K"),("settings","Settings…","CmdOrCtrl+,"),("find-work","Find Work…","CmdOrCtrl+P"),("page-search","Search Current Page…","CmdOrCtrl+F"),("chat-terminal","Toggle Chat Terminal","CmdOrCtrl+J")] {
                            submenu.append(&tauri::menu::MenuItem::with_id(app.handle(),id,label,true,Some(key))?)?;
                        }
                    }
                    if submenu.text()? == "File" {
                        for (id, label, key) in [
                            ("agent-new", "New Agent Tab", "CmdOrCtrl+T"),
                            ("agent-close", "Close Agent Tab", "CmdOrCtrl+W"),
                            ("agent-reopen", "Reopen Closed Agent Tab", "CmdOrCtrl+Shift+T"),
                            ("agent-rename", "Rename Agent Tab", "CmdOrCtrl+R"),
                        ] {
                            submenu.append(&tauri::menu::MenuItem::with_id(app.handle(), id, label, true, Some(key))?)?;
                        }
                    }
                }
            }
            app.on_menu_event(|app, event| {
                let id=event.id().as_ref();
                if !["agent-new","agent-close","agent-reopen","agent-rename","toggle-left","toggle-right","commands","find-work","page-search","settings","chat-terminal"].contains(&id){return;}
                let script=format!("window.dispatchEvent(new CustomEvent('mydash-configured-command',{{detail:{}}}))",serde_json::json!(id));
                if let Some(window) = app.get_webview_window("main") { let _ = window.eval(&script); }
            });
            app.set_menu(menu)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("Could not initialize truffle");
    app.run(|app, event| {
        if let tauri::RunEvent::Reopen { .. } = event {
            show(app)
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_accepts_existing_directories_and_rejects_files_and_relative_paths() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("worktree with spaces ; $(ignored)");
        std::fs::create_dir(&folder).unwrap();
        assert_eq!(zed_directory(folder.to_str().unwrap()).unwrap(), folder.canonicalize().unwrap());
        assert!(zed_directory("relative/repo").is_err());
        assert!(zed_directory(root.path().join("missing").to_str().unwrap()).is_err());
        let file = root.path().join("file.txt");
        std::fs::write(&file, "text").unwrap();
        assert!(zed_directory(file.to_str().unwrap()).is_err());
    }
    #[test]
    fn external_links_do_not_launch_shell_or_file_handlers() {
        let service = LocalHttpService::new("http://127.0.0.1:8765").unwrap();
        for value in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://user:secret@example.com",
            "mydash-artifact://localhost/../secret",
        ] {
            assert!(external_url(&tauri::Url::parse(value).unwrap(), &service).is_err());
        }
        assert!(external_url(
            &tauri::Url::parse("https://github.com/acme/api/pull/1").unwrap(),
            &service
        )
        .is_ok());
    }
}
