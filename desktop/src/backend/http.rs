//! Loopback API and authenticated agent inbox. No UI-specific persistence.
use super::chat_preparation::{self, Kind};
use super::{
    arr, checked, domain, home, json, now, sessions, sid, text, uuid, Error, PathBuf, Repository,
    Result, Value,
};
use super::{artifacts, workflows};
use crate::backend::operations::Caller;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    fmt::Write as _,
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
const MAX_BODY: usize = 128 * 1024;
const CSP:&str="default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'";
/// Replace `path` atomically: readers see the old or the new bytes, never a partial file.
pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| Error::new("Write target has no file name."))?;
    let temp = path.with_file_name(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    Ok(written?)
}
type Reply = tiny_http::Response<std::io::Cursor<Vec<u8>>>;
fn json_response(status: u16, value: &Value) -> Reply {
    response(
        status,
        serde_json::to_vec(value).unwrap_or_default(),
        "application/json",
        false,
    )
}
fn response(
    status: u16,
    bytes: Vec<u8>,
    kind: &str,
    artifact: bool,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let mut r = tiny_http::Response::from_data(bytes).with_status_code(status);
    for (k, v) in [
        ("Content-Type", kind),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "no-referrer"),
        (
            "Content-Security-Policy",
            if artifact {
                "sandbox allow-popups allow-popups-to-escape-sandbox; default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'self'"
            } else {
                CSP
            },
        ),
    ] {
        r.add_header(tiny_http::Header::from_bytes(k, v).unwrap());
    }
    if !artifact {
        r.add_header(tiny_http::Header::from_bytes("X-Frame-Options", "DENY").unwrap());
    }
    r
}
fn header<'a>(r: &'a tiny_http::Request, name: &str) -> &'a str {
    r.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map_or("", |h| h.value.as_str())
}
fn equal(a: &str, b: &str) -> bool {
    let mut mac = Hmac::<Sha256>::new_from_slice(b"truffle-comparison").unwrap();
    mac.update(b.as_bytes());
    let expected = mac.finalize().into_bytes();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"truffle-comparison").unwrap();
    mac.update(a.as_bytes());
    mac.verify_slice(&expected).is_ok()
}
fn handle(
    mut req: tiny_http::Request,
    repo: &Arc<Repository>,
    authority: &str,
    csrf: &str,
    token: &str,
) {
    let result = (|| -> Result<tiny_http::Response<std::io::Cursor<Vec<u8>>>> {
        if header(&req, "Host") != authority {
            return Ok(json_response(403, &json!({"error":"Invalid host."})));
        }
        let path = req.url().split('?').next().unwrap_or("").to_string();
        let origin = format!("http://{authority}");
        if req.method() == &tiny_http::Method::Get {
            return get_route(repo, &path, csrf);
        }
        if req.method() != &tiny_http::Method::Post {
            return Ok(json_response(405, &json!({"error":"Method not allowed."})));
        }
        let bridge = path == "/api/agent/register";
        let authorized = if bridge {
            header(&req, "Origin").is_empty()
                && equal(header(&req, "Authorization"), &format!("Bearer {token}"))
        } else {
            header(&req, "Origin") == origin && equal(header(&req, "X-CSRF-Token"), csrf)
        };
        if !authorized || header(&req, "Content-Type") != "application/json" {
            return Ok(json_response(
                403,
                &json!({"error":"Request rejected. Reload this dashboard and try again."}),
            ));
        }
        let body_limit = if path == "/api/app-chat/send" {
            2 * 1024 * 1024
        } else {
            MAX_BODY
        };
        if req.body_length().is_some_and(|n| n > body_limit) {
            return Ok(json_response(413, &json!({"error":"Request too large."})));
        }
        let mut bytes = vec![];
        req.as_reader()
            .take(body_limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > body_limit || bytes.is_empty() {
            return Ok(json_response(
                413,
                &json!({"error":"Request too large or empty."}),
            ));
        }
        let payload: Value = serde_json::from_slice(&bytes)?;
        if !payload.is_object() {
            return Err(Error::new("Expected a JSON object."));
        }
        post_route(repo, &path, &payload)
    })();
    let response = match result {
        Ok(r) => r,
        Err(e) => {
            let mut data = e.data;
            if !data.is_object() {
                data = json!({});
            }
            data["error"] = json!(e.message);
            json_response(e.status, &data)
        }
    };
    let _ = req.respond(response);
}
fn get_route(repo: &Arc<Repository>, path: &str, csrf: &str) -> Result<Reply> {
    Ok(match path {
        "/api/health" => json_response(
            200,
            &json!({"service":"truffle","apiVersion":1,"storageVersion":1,"backend":"rust"}),
        ),
        "/api/state" => {
            let mut state = repo.snapshot(false);
            state["csrf"] = json!(csrf);
            json_response(200, &state)
        }
        "/api/app-chat" => json_response(200, &repo.app_chat.view(repo)),
        "/api/app-chat/skill" => json_response(200, &super::app_chat::guide()),
        "/api/themes" => json_response(200, &repo.settings.themes()?),
        "/api/settings" => json_response(200, &browser(repo, "settings.read", &json!({}))?),
        "/api/config-status" => json_response(200, &super::config_status::view(repo)),
        "/api/quick-actions" => {
            json_response(200, &browser(repo, "quick-actions.list", &json!({}))?)
        }
        "/api/export" => json_response(200, &repo.snapshot(true)),
        "/api/agents" => json_response(200, &repo.inventory()),
        "/api/gcp-auth" => json_response(200, &repo.gcp_auth.view()),
        "/api/views" => json_response(200, &super::views::view(repo)?),
        "/api/page-settings" => json_response(200, &super::page_settings::view(repo)?),
        "/api/feedback" => json_response(200, &super::feedback::view(repo)?),
        "/api/guidance" => json_response(200, &super::guidance::view(repo)?),
        "/api/onboarding" => json_response(200, &super::onboarding::view(repo)),
        "/api/commands" => json_response(200, &super::commands::catalog()),
        "/api/logs" => json_response(200, &repo.logs()),
        "/api/sharing" => json_response(200, &browser(repo, "sharing.state", &json!({}))?),
        "/api/library" => {
            let s = repo.state.lock().unwrap();
            json_response(200, &artifacts::catalog(repo, &s)?)
        }
        p if p.starts_with("/artifacts/") => {
            let (bytes, kind) = artifacts::read(repo, p.trim_start_matches("/artifacts/"))?;
            response(200, bytes, &kind, true)
        }
        p if p.starts_with("/linear/") => linear_redirect(repo, p.trim_start_matches("/linear/"))?,
        "/favicon.ico" => response(204, vec![], "image/x-icon", false),
        p => static_asset(p)?,
    })
}
fn linear_redirect(repo: &Repository, id: &str) -> Result<Reply> {
    if !regex::Regex::new(r"^[A-Z][A-Z0-9]*-[0-9]+$")
        .unwrap()
        .is_match(id)
    {
        return Err(Error::new("Invalid Linear identifier."));
    }
    let cached = {
        let s = repo.state.lock().unwrap();
        domain::catalog(&s)
            .values()
            .find(|x| x["source"] == "linear" && x["reference"] == id)
            .map(|x| text(x, "url").to_owned())
            .or_else(|| s["linearLinks"][id].as_str().map(str::to_owned))
    };
    let url = if let Some(v) = cached {
        v
    } else {
        let v = repo.providers.linear_url(id)?;
        repo.transaction(|s| {
            s["linearLinks"][id] = json!(v);
            Ok(())
        })?;
        v
    };
    let mut response = response(302, vec![], "text/plain", false);
    response.add_header(tiny_http::Header::from_bytes("Location", url).unwrap());
    Ok(response)
}
fn static_asset(p: &str) -> Result<Reply> {
    if let Some(name) = p.strip_prefix("/vendor/diffs/") {
        if !has_extension(name, "js")
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        {
            return Ok(json_response(404, &json!({"error":"Not found."})));
        }
        let bytes = super::assets::asset(&format!("vendor/diffs/{name}"))
            .ok_or_else(|| Error::new("Asset unavailable."))?
            .to_vec();
        return Ok(response(
            200,
            bytes,
            "text/javascript; charset=utf-8",
            false,
        ));
    }
    let Some(file) = asset_file(p) else {
        return Ok(json_response(404, &json!({"error":"Not found."})));
    };
    let kind = if has_extension(file, "html") {
        "text/html; charset=utf-8"
    } else if has_extension(file, "css") {
        "text/css"
    } else {
        "text/javascript; charset=utf-8"
    };
    Ok(response(
        200,
        super::assets::asset(file)
            .ok_or_else(|| Error::new("Asset unavailable."))?
            .to_vec(),
        kind,
        false,
    ))
}
fn asset_file(path: &str) -> Option<&'static str> {
    Some(match path {
        "/" | "/index.html" => "index.html",
        "/app.js" => "app.js",
        "/settings.js" => "settings.js",
        "/configuration.js" => "configuration.js",
        "/settings-profile.js" => "settings-profile.js",
        "/settings-profile.css" => "settings-profile.css",
        "/product-tools.js" => "product-tools.js",
        "/product-tools.css" => "product-tools.css",
        "/onboarding.js" => "onboarding.js",
        "/guidance.js" => "guidance.js",
        "/page-settings.js" => "page-settings.js",
        "/page-settings.css" => "page-settings.css",
        "/views.js" => "views.js",
        "/views.css" => "views.css",
        "/feedback.js" => "feedback.js",
        "/chat-attachments.js" => "chat-attachments.js",
        "/chat-results.js" => "chat-results.js",
        "/chat-results.css" => "chat-results.css",
        "/guidance.css" => "guidance.css",
        "/onboarding.css" => "onboarding.css",
        "/presentation.js" => "presentation.js",
        "/app-chat.js" => "app-chat.js",
        "/skill-picker.js" => "skill-picker.js",
        "/skill-picker.css" => "skill-picker.css",
        "/app-chat.css" => "app-chat.css",
        "/settings.css" => "settings.css",
        "/connection-setup.js" => "connection-setup.js",
        "/connection-setup.css" => "connection-setup.css",
        "/ui-controls.css" => "ui-controls.css",
        "/command-palette.css" => "command-palette.css",
        "/client.js" => "client.js",
        "/desktop.js" => "desktop.js",
        "/performance.js" => "performance.js",
        "/workspace.js" => "workspace.js",
        "/resources.js" => "resources.js",
        "/artifact-copy.js" => "artifact-copy.js",
        "/sharing.js" => "sharing.js",
        "/gcp-auth.js" => "gcp-auth.js",
        "/restart.js" => "restart.js",
        "/view-agent.js" => "view-agent.js",
        "/github-actions.js" => "github-actions.js",
        "/git-diff.js" => "git-diff.js",
        "/ux.js" => "ux.js",
        "/command-palette.js" => "command-palette.js",
        "/session-dispatch.js" => "session-dispatch.js",
        "/skills-editor.js" => "skills-editor.js",
        "/chat-preparation.js" => "chat-preparation.js",
        "/chat-preparation.css" => "chat-preparation.css",
        "/diff-review.js" => "diff-review.js",
        "/workspace.css" => "workspace.css",
        "/vendor/xterm.js" => "vendor/xterm.js",
        "/vendor/addon-fit.js" => "vendor/addon-fit.js",
        "/vendor/xterm.css" => "vendor/xterm.css",
        "/sessions.js" => "sessions.js",
        "/features.js" => "features.js",
        "/newsletters.js" => "newsletters.js",
        "/markdown.js" => "markdown.js",
        "/vendor/markdown-it.min.js" => "vendor/markdown-it.min.js",
        "/style.css" => "style.css",
        _ => return None,
    })
}
pub(super) fn post_route(repo: &Arc<Repository>, path: &str, payload: &Value) -> Result<Reply> {
    if let Some(value) = service_post(repo, path, payload)? {
        return Ok(json_response(200, &value));
    }
    if let Some(value) = session_post(repo, path, payload)? {
        return Ok(json_response(200, &value));
    }
    if path.starts_with("/api/terminal/") {
        return Ok(json_response(200, &terminal_post(repo, path, payload)?));
    }
    state_post(repo, path, payload)
}
fn service_post(repo: &Arc<Repository>, path: &str, payload: &Value) -> Result<Option<Value>> {
    Ok(Some(match path {
        "/api/connections/reload" => browser(repo, "connections.reload", payload)?,
        "/api/views" => super::views::update(repo, payload)?,
        "/api/page-settings" => super::page_settings::update(repo, payload)?,
        "/api/feedback" => super::feedback::update(repo, payload)?,
        "/api/guidance" => super::guidance::update(repo, payload)?,
        "/api/onboarding" => super::onboarding::update(repo, payload)?,
        "/api/shell-terminal/open" => super::chat_terminal::open(repo, payload)?,
        p if p.starts_with("/api/shell-terminal/") => shell_terminal(repo, p, payload)?,
        "/api/commands/invoke" => super::commands::invoke(repo, payload, Caller::Browser)?,
        "/api/skills/catalog" => skills_catalog(repo, payload)?,
        "/api/app-chat/send" => repo.app_chat.start(repo, payload)?,
        "/api/app-chat/new" => repo.app_chat.new_chat(repo)?,
        "/api/app-chat/cancel" => repo.app_chat.cancel(),
        "/api/settings/preview" => browser(repo, "settings.preview", payload)?,
        "/api/settings" => browser(repo, "settings.apply", payload)?,
        "/api/quick-actions/run" => browser(repo, "quick-actions.run", payload)?,
        _ => return Ok(None),
    }))
}
fn skills_catalog(repo: &Repository, payload: &Value) -> Result<Value> {
    let mut projects = Vec::new();
    if !text(payload, "cwd").is_empty() {
        let p = PathBuf::from(text(payload, "cwd"));
        if p.is_absolute() && p.is_dir() {
            projects.push(p);
        }
    }
    let records = sessions::all(&*repo.state.lock().map_err(|_| Error::io())?).to_vec();
    for row in records {
        let p = PathBuf::from(text(&row, "cwd"));
        if p.is_absolute() && p.is_dir() && !projects.contains(&p) {
            projects.push(p);
        }
        for tree in arr(&row["worktrees"]) {
            let p = PathBuf::from(text(&tree, "path"));
            if p.is_absolute() && p.is_dir() && !projects.contains(&p) {
                projects.push(p);
            }
        }
        if projects.len() >= 20 {
            break;
        }
    }
    Ok(super::skills_catalog::discover(
        &home(),
        &projects,
        &repo.settings.config_directory().join("skills"),
    ))
}
fn session_post(repo: &Arc<Repository>, path: &str, payload: &Value) -> Result<Option<Value>> {
    Ok(Some(match path {
        p if p.starts_with("/api/sharing/") => sharing(repo, p, payload)?,
        "/api/session/links" => {
            if let Some(context) = payload.get("context") {
                super::session_context::replace_receipt(repo, text(payload, "id"), context)?
            } else {
                super::session_context::read(repo, text(payload, "id"))?
            }
        }
        "/api/session/review-draft" => super::reviews::action(repo, &super::reviews::Git, payload)?,
        "/api/session/ci-investigation" => super::session_ci::investigation(repo, payload)?,
        "/api/session/ci-refresh" => browser(
            repo,
            "chats.ci.refresh",
            &json!({"recordId": payload["id"]}),
        )?,
        "/api/session/git-file" => super::git_diff::file_for_session(
            repo,
            text(payload, "id"),
            text(payload, "path"),
            text(payload, "cwd"),
        )?,
        "/api/session/git-diff" => {
            super::git_diff::for_session(repo, text(payload, "id"), text(payload, "cwd"))?
        }
        _ => return Ok(None),
    }))
}
fn shell_terminal(repo: &Repository, path: &str, payload: &Value) -> Result<Value> {
    use super::chat_terminal::{action, Action};
    let operation = match path {
        "/api/shell-terminal/poll" => Action::Poll,
        "/api/shell-terminal/input" => Action::Input,
        "/api/shell-terminal/resize" => Action::Resize,
        "/api/shell-terminal/close" => Action::Close,
        _ => return Err(Error::new("Unknown terminal command.")),
    };
    action(repo, operation, payload)
}
fn sharing(repo: &Arc<Repository>, path: &str, payload: &Value) -> Result<Value> {
    let name = match path {
        "/api/sharing/job" => "sharing.job",
        "/api/sharing/config" => "sharing.configure",
        "/api/sharing/refresh" => "sharing.refresh",
        "/api/sharing/dismiss" => "sharing.dismiss",
        "/api/sharing/prepare" => "sharing.prepare",
        "/api/sharing/send" => "sharing.send",
        "/api/sharing/preview" => "sharing.preview",
        "/api/sharing/import" => "sharing.import",
        _ => return Err(Error::new("Sharing operation unavailable.")),
    };
    browser(repo, name, payload)
}
fn browser(repo: &Arc<Repository>, name: &str, args: &Value) -> Result<Value> {
    super::operations::run(repo, Caller::Browser, name, args)
}
fn terminal_post(repo: &Repository, path: &str, payload: &Value) -> Result<Value> {
    let id = sid(text(payload, "id"))?;
    Ok(match path {
        "/api/terminal/poll" => repo.terminals.poll(
            &id,
            payload["cursor"].as_u64().unwrap_or(0),
            text(payload, "generation"),
        )?,
        "/api/terminal/close" => {
            repo.terminals.close(&id, text(payload, "generation"))?;
            json!({"message":"Terminal closed. The saved conversation remains in History."})
        }
        "/api/terminal/input" => {
            let data = payload["data"]
                .as_str()
                .ok_or_else(|| Error::new("Expected terminal input."))?;
            repo.terminals.send(&id, data)?;
            json!({})
        }
        "/api/terminal/resize" => {
            repo.terminals.resize(&id, payload)?;
            json!({})
        }
        _ => return Err(Error::new("Terminal operation unavailable.")),
    })
}
fn state_post(repo: &Arc<Repository>, path: &str, payload: &Value) -> Result<Reply> {
    let mut extra = json!({});
    let saved = match path {
        "/api/agent/register" => {
            let record = workflows::register(repo, payload)?;
            return Ok(json_response(200, &workflows::registration_reply(&record)));
        }
        "/api/edit" => {
            repo.edit(payload)?;
            None
        }
        "/api/github/pr" => {
            super::github::action(repo, payload)?;
            None
        }
        "/api/github/reviews/seen" => {
            super::github::seen(repo, payload)?;
            None
        }
        "/api/session/prepare" => Some(if text(payload, "id").is_empty() {
            chat_preparation::save(repo, Kind::New, payload)?
        } else {
            super::chat_edit::legacy(repo, payload)?
        }),
        "/api/session/from-view" => Some(chat_preparation::save(repo, Kind::View, payload)?),
        "/api/session/quick" => Some(chat_preparation::save(repo, Kind::Quick, payload)?),
        "/api/session/pr-review" => Some(chat_preparation::save(repo, Kind::PrReview, payload)?),
        "/api/session/review" => Some(chat_preparation::save(repo, Kind::Review, payload)?),
        "/api/session/newsletter" => Some(chat_preparation::save(repo, Kind::Skill, payload)?),
        "/api/skills/run" => Some(super::scheduler::execute(
            repo,
            text(payload, "presetId"),
            false,
            None,
        )?),
        "/api/skills/preset" => {
            super::scheduler::save_preset(repo, payload)?;
            None
        }
        "/api/skills/schedule" => {
            super::scheduler::save_schedule(repo, payload)?;
            None
        }
        "/api/skills/read" => {
            repo.transaction(|s| {
                s["skillsReadAt"] = json!(now());
                Ok(())
            })?;
            None
        }
        "/api/session/recover" => Some(chat_preparation::save(repo, Kind::Recover, payload)?),
        "/api/session/open" => {
            open_session(repo, payload, &mut extra)?;
            None
        }
        "/api/session/context" => {
            return Ok(json_response(
                200,
                &json!({"context":workflows::context(repo,text(payload,"id"))?}),
            ))
        }
        "/api/gcp-auth/start" => return Ok(json_response(200, &repo.gcp_auth.start(repo.demo)?)),
        "/api/gcp-auth/cancel" => return Ok(json_response(200, &repo.gcp_auth.cancel())),
        "/api/sync" => {
            let receipt = browser(
                repo,
                "sources.refresh",
                &json!({"force": payload["force"] == true}),
            )?;
            extra["message"] = receipt["message"].clone();
            extra["refreshReceipt"] = receipt;
            None
        }
        "/api/sentry/environment" => {
            super::source_sync::environment(repo, &checked(payload, "environment", 100, true)?)?;
            None
        }
        _ => return Ok(json_response(404, &json!({"error":"Not found."}))),
    };
    let mut state = repo.snapshot(false);
    if let Some(record) = saved {
        state["savedSessionId"] = record["id"].clone();
    }
    for (k, v) in extra.as_object().unwrap() {
        state[k] = v.clone();
    }
    Ok(json_response(200, &state))
}
fn open_session(repo: &Arc<Repository>, payload: &Value, extra: &mut Value) -> Result<()> {
    let fields = payload
        .as_object()
        .ok_or_else(|| Error::new("Open arguments must be an object."))?;
    if fields
        .keys()
        .any(|key| !["id", "target"].contains(&key.as_str()))
    {
        return Err(Error::new(
            "Open accepts only a saved chat ID and destination intent.",
        ));
    }
    let mut args = json!({"recordId":payload["id"]});
    if let Some(target) = payload.get("target") {
        args["target"] = target.clone();
    }
    let receipt = browser(repo, "chats.open", &args)?;
    extra["message"] = receipt["message"].clone();
    if receipt["status"] == "accepted" && receipt["target"] == "embedded" {
        extra["embedded"] = json!(true);
    }
    if receipt["status"] == "preview" {
        extra["preview"] = json!(true);
    }
    extra["openReceipt"] = receipt;
    Ok(())
}
/// Same as `name.ends_with(".{extension}")`, including for a bare `.{extension}` name.
fn has_extension(name: &str, extension: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, found)| found == extension)
}
fn ascii_json_string(value: &str) -> String {
    let encoded = serde_json::to_string(value).unwrap();
    let mut result = String::new();
    for c in encoded.chars() {
        if c as u32 > 127 {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                let _ = write!(result, "\\u{unit:04x}");
            }
        } else {
            result.push(c);
        }
    }
    result
}
pub(super) fn canonical(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys = m.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|k| format!("{}:{}", ascii_json_string(k), canonical(&m[k])))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        Value::String(s) => ascii_json_string(s),
        _ => v.to_string(),
    }
}
pub(super) fn inbox_once(repo: &Arc<Repository>, inbox: &Path, token: &str) {
    let Ok(entries) = std::fs::read_dir(inbox) else {
        return;
    };
    for entry in entries.filter_map(std::result::Result::ok).take(100) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".result.json") || has_extension(&name, "tmp") {
            if std::fs::symlink_metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 300)
            {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        if !name.ends_with(".request.json") {
            continue;
        }
        let mut reply_needed = true;
        let result = (|| -> Result<Value> {
            let info = std::fs::symlink_metadata(&path)?;
            if !info.is_file()
                || info.uid() != unsafe { libc::getuid() }
                || info.len() > MAX_BODY as u64
            {
                return Err(Error::new("Invalid local registration envelope."));
            }
            let envelope: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
            let data = &envelope["data"];
            let nonce = sid(text(data, "nonce"))?;
            if name != format!("{nonce}.request.json")
                || (now() - data["time"].as_f64().unwrap_or(0.0)).abs() > 180.0
            {
                return Err(Error::new("Expired local registration envelope."));
            }
            let expected = hex_decode(text(&envelope, "signature"))?;
            let mut mac = Hmac::<Sha256>::new_from_slice(token.as_bytes()).unwrap();
            mac.update(canonical(data).as_bytes());
            mac.verify_slice(&expected)
                .map_err(|_| Error::new("Invalid registration signature."))?;
            reply_needed = data["reply"] != false;
            let record = workflows::register(repo, &data["payload"])?;
            Ok(workflows::registration_reply(&record))
        })();
        let reply = match result {
            Ok(v) => v,
            Err(e) => json!({"error":e.message}),
        };
        let result_path = inbox.join(name.replace(".request.json", ".result.json"));
        if reply_needed && !result_path.exists() {
            let _ = private_write(&result_path, &serde_json::to_vec(&reply).unwrap());
        }
        let _ = std::fs::remove_file(path);
        break;
    }
}
fn hex_decode(value: &str) -> Result<Vec<u8>> {
    if value.len() != 64 {
        return Err(Error::new("Invalid signature."));
    }
    (0..value.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&value[i..i + 2], 16).map_err(|_| Error::new("Invalid signature."))
        })
        .collect()
}
pub fn serve(
    root: PathBuf,
    path: Option<PathBuf>,
    port: u16,
    demo: bool,
    terminal_demo: Option<String>,
) -> Result<()> {
    let repo = Repository::open(root, path, demo)?;
    if let Some(id) = terminal_demo {
        if !demo {
            return Err(Error::new(
                "--terminal-demo requires --demo and an isolated data file.",
            ));
        }
        for id in id.split(',') {
            let record = sessions::get(&repo.state.lock().unwrap(), id)?.clone();
            repo.terminals.open_demo(&record)?;
        }
    }
    let server = tiny_http::Server::http(("127.0.0.1", port))
        .map_err(|_| Error::new("Cannot bind the truffle loopback port."))?;
    let port = server.server_addr().to_ip().unwrap().port();
    let authority = format!("127.0.0.1:{port}");
    let token = format!("{}{}", uuid(), uuid());
    let csrf = format!("{}{}", uuid(), uuid());
    let inbox = PathBuf::from("/tmp").join(format!("truffle-agent-{}", uuid()));
    std::fs::create_dir(&inbox)?;
    std::fs::set_permissions(&inbox, std::fs::Permissions::from_mode(0o700))?;
    let runtime = repo.data.parent().unwrap().join(if demo {
        "demo-runtime.json"
    } else {
        "runtime.json"
    });
    private_write(
        &runtime,
        &serde_json::to_vec(
            &json!({"url":format!("http://{authority}"),"token":token,"inbox":inbox,"backend":"rust","pid":std::process::id(),"root":repo.root,"data":repo.data,"builtAt":env!("TRUFFLE_BUILD_TIME")}),
        )?,
    )?;
    let sync_repo = repo.clone();
    std::thread::spawn(move || loop {
        let _ = super::source_sync::request(&sync_repo, false);
        std::thread::sleep(std::time::Duration::from_secs(10));
    });
    let scheduler_repo = repo.clone();
    std::thread::spawn(move || loop {
        let _ = super::scheduler::tick(&scheduler_repo);
        std::thread::sleep(std::time::Duration::from_secs(10));
    });
    let share_repo = repo.clone();
    std::thread::spawn(move || loop {
        super::sharing::tick(&share_repo);
        std::thread::sleep(std::time::Duration::from_secs(15));
    });
    let inbox_repo = repo.clone();
    let inbox_token = token.clone();
    std::thread::spawn(move || loop {
        inbox_once(&inbox_repo, &inbox, &inbox_token);
        std::thread::sleep(std::time::Duration::from_millis(500));
    });
    println!(
        "truffle Rust service → http://{authority}{}",
        if demo {
            " (preview; no external requests)"
        } else {
            ""
        }
    );
    let active = Arc::new(AtomicUsize::new(0));
    for request in server.incoming_requests() {
        if active.load(Ordering::Relaxed) >= 32 {
            let _ = request.respond(json_response(
                429,
                &json!({"error":"Too many local requests. Retry shortly."}),
            ));
            continue;
        }
        active.fetch_add(1, Ordering::Relaxed);
        let count = active.clone();
        let repo = repo.clone();
        let authority = authority.clone();
        let token = token.clone();
        let csrf = csrf.clone();
        std::thread::spawn(move || {
            handle(request, &repo, &authority, &csrf, &token);
            count.fetch_sub(1, Ordering::Relaxed);
        });
    }
    Ok(())
}

#[cfg(test)]
mod private_write_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_write_replaces_whole_files_privately_and_leaves_no_temp_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("doc.json");
        std::fs::write(&path, b"old and longer").unwrap();
        private_write(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let blocked = temp.path().join("dir");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("child"), b"").unwrap();
        assert!(private_write(&blocked, b"x").is_err());
        let mut names = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, ["dir", "doc.json"]);
    }
}
