//! Loopback API and authenticated agent inbox. No UI-specific persistence.
use super::*;
use super::{artifacts, workflows};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
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
pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(())
}
fn claim_sentry_environment(repo:&Repository,environment:&str)->Result<bool>{
    repo.transaction(|s| {
        if s["sources"]["sentry"]["disabled"] == true {return Err(Error::new("Sentry synchronization is paused. Enable the connection before requesting an environment. Cached results remain available."));}
        let caches = s["sentryEnvironments"].as_object().unwrap();
        if !caches.contains_key(environment) && caches.len() >= 20 {
            return Err(Error::new("Up to 20 environment caches are supported."));
        }
        let source = &s["sources"]["sentry"];
        if source["failures"].as_u64().unwrap_or(0) > 0
            && source["nextAttempt"].as_f64().unwrap_or(0.0) > now() {
            return Err(Error::new("Sentry is waiting for its connection retry window. Cached results remain available."));
        }
        if repo.demo || caches.values().any(|v| v["running"] == true)
            || s["sentryEnvNextAttempt"].as_f64().unwrap_or(0.0) > now()
            || s["sentryEnvironments"][environment]["nextAttempt"].as_f64().unwrap_or(0.0) > now() {
            return Ok(false);
        }
        if s["sentryEnvironments"][environment].is_null() {
            s["sentryEnvironments"][environment] = json!({});
        }
        s["sentryEnvironments"][environment]["running"] = json!(true);
        s["sentryEnvironments"][environment]["nextAttempt"] = json!(now()+300.0);
        s["sentryEnvNextAttempt"] = json!(now()+15.0);
        Ok(true)
    })
}
fn json_response(status: u16, value: Value) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    response(
        status,
        serde_json::to_vec(&value).unwrap_or_default(),
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
        .map(|h| h.value.as_str())
        .unwrap_or("")
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
            return Ok(json_response(403, json!({"error":"Invalid host."})));
        }
        let path = req.url().split('?').next().unwrap_or("").to_string();
        let origin = format!("http://{authority}");
        if req.method() == &tiny_http::Method::Get {
            return Ok(match path.as_str() {
                "/api/health" => json_response(
                    200,
                    json!({"service":"truffle","apiVersion":1,"storageVersion":1,"backend":"rust"}),
                ),
                "/api/state" => {
                    let mut state = repo.snapshot(false);
                    state["csrf"] = json!(csrf);
                    json_response(200, state)
                }
                "/api/app-chat" => json_response(200,repo.app_chat.view(repo)),
                "/api/app-chat/skill" => json_response(200,super::app_chat::guide()),
                "/api/themes" => json_response(200,repo.settings.themes()?),
                "/api/settings" => json_response(200,repo.settings.view()),
                "/api/config-status" => json_response(200,super::config_status::view(repo)),
                "/api/quick-actions" => json_response(200,repo.quick_actions.view()),
                "/api/export" => json_response(200, repo.snapshot(true)),
                "/api/agents" => json_response(200, repo.inventory()),
                "/api/gcp-auth" => json_response(200, repo.gcp_auth.view()),
                "/api/views"=>json_response(200,super::views::view(repo)?),
                "/api/page-settings"=>json_response(200,super::page_settings::view(repo)?),
                "/api/feedback"=>json_response(200,super::feedback::view(repo)?),
                "/api/guidance"=>json_response(200,super::guidance::view(repo)?),
                "/api/onboarding"=>json_response(200,super::onboarding::view(repo)),
                "/api/commands" => json_response(200,super::commands::catalog()),
                "/api/logs" => json_response(200, repo.logs()),
                "/api/sharing" => json_response(200, super::sharing::view(repo)),
                "/api/library" => {
                    let s = repo.state.lock().unwrap();
                    json_response(200, artifacts::catalog(repo, &s)?)
                }
                p if p.starts_with("/artifacts/") => {
                    let (bytes, kind) = artifacts::read(repo, p.trim_start_matches("/artifacts/"))?;
                    response(200, bytes, &kind, true)
                }
                p if p.starts_with("/linear/") => {
                    let id = p.trim_start_matches("/linear/");
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
                    let url = match cached {
                        Some(v) => v,
                        None => {
                            let v = repo.providers.linear_url(id)?;
                            repo.transaction(|s| {
                                s["linearLinks"][id] = json!(v);
                                Ok(())
                            })?;
                            v
                        }
                    };
                    let mut response = response(302, vec![], "text/plain", false);
                    response.add_header(tiny_http::Header::from_bytes("Location", url).unwrap());
                    response
                }
                "/favicon.ico" => response(204, vec![], "image/x-icon", false),
                p => {
                    if let Some(name)=p.strip_prefix("/vendor/diffs/") {
                        if !name.ends_with(".js")||!name.chars().all(|c|c.is_ascii_alphanumeric()||"-_.".contains(c)){return Ok(json_response(404,json!({"error":"Not found."})));}
                        let bytes=super::assets::asset(&format!("vendor/diffs/{name}")).ok_or(Error::new("Asset unavailable."))?.to_vec();
                        return Ok(response(200,bytes,"text/javascript; charset=utf-8",false));
                    }
                    let file = match p {
                        "/" | "/index.html" => "index.html",
                        "/app.js" => "app.js",
                        "/settings.js" => "settings.js",
                        "/configuration.js" => "configuration.js",
                        "/settings-profile.js" => "settings-profile.js",
                        "/settings-profile.css" => "settings-profile.css",
                        "/product-tools.js"=>"product-tools.js",
                        "/product-tools.css"=>"product-tools.css",
                        "/onboarding.js"=>"onboarding.js",
                        "/guidance.js"=>"guidance.js",
                        "/page-settings.js"=>"page-settings.js",
                        "/page-settings.css"=>"page-settings.css",
                        "/views.js"=>"views.js",
                        "/views.css"=>"views.css",
                        "/feedback.js"=>"feedback.js",
                        "/chat-attachments.js"=>"chat-attachments.js",
                        "/chat-results.js"=>"chat-results.js",
                        "/chat-results.css"=>"chat-results.css",
                        "/guidance.css"=>"guidance.css",
                        "/onboarding.css"=>"onboarding.css",
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
                        _ => return Ok(json_response(404, json!({"error":"Not found."}))),
                    };
                    let kind = if file.ends_with(".html") {
                        "text/html; charset=utf-8"
                    } else if file.ends_with(".css") {
                        "text/css"
                    } else {
                        "text/javascript; charset=utf-8"
                    };
                    response(
                        200,
                        super::assets::asset(file).ok_or(Error::new("Asset unavailable."))?.to_vec(),
                        kind,
                        false,
                    )
                }
            });
        }
        if req.method() != &tiny_http::Method::Post {
            return Ok(json_response(405, json!({"error":"Method not allowed."})));
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
                json!({"error":"Request rejected. Reload this dashboard and try again."}),
            ));
        }
        let body_limit=if path=="/api/app-chat/send"{2*1024*1024}else{MAX_BODY};
        if req.body_length().is_some_and(|n| n > body_limit) {
            return Ok(json_response(413, json!({"error":"Request too large."})));
        }
        let mut bytes = vec![];
        req.as_reader()
            .take(body_limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > body_limit || bytes.is_empty() {
            return Ok(json_response(
                413,
                json!({"error":"Request too large or empty."}),
            ));
        }
        let payload: Value = serde_json::from_slice(&bytes)?;
        if !payload.is_object() {
            return Err(Error::new("Expected a JSON object."));
        }
        if path=="/api/connections/reload" {return Ok(json_response(200,super::commands::invoke(repo,&json!({"name":"connections.reload","args":payload}),false)?));}
        if path=="/api/views"{return Ok(json_response(200,super::views::update(repo,&payload)?));}
        if path=="/api/page-settings"{return Ok(json_response(200,super::page_settings::update(repo,&payload)?));}
        if path=="/api/feedback"{return Ok(json_response(200,super::feedback::update(repo,&payload)?));}
        if path=="/api/guidance"{return Ok(json_response(200,super::guidance::update(repo,&payload)?));}
        if path=="/api/onboarding"{return Ok(json_response(200,super::onboarding::update(repo,&payload)?));}
        if path.starts_with("/api/shell-terminal/"){return Ok(json_response(200,super::chat_terminal::action(repo,&path,&payload)?));}
        if path=="/api/commands/invoke"{return Ok(json_response(200,super::commands::invoke(repo,&payload,false)?));}
        if path=="/api/skills/catalog" {
            let mut projects=Vec::new();if !text(&payload,"cwd").is_empty(){let p=PathBuf::from(text(&payload,"cwd"));if p.is_absolute()&&p.is_dir(){projects.push(p);}}
            let state=repo.state.lock().map_err(|_|Error::io())?;for row in arr(&state["sessions"]){let p=PathBuf::from(text(&row,"cwd"));if p.is_absolute()&&p.is_dir()&&!projects.contains(&p){projects.push(p);}for tree in arr(&row["worktrees"]){let p=PathBuf::from(text(&tree,"path"));if p.is_absolute()&&p.is_dir()&&!projects.contains(&p){projects.push(p);}}if projects.len()>=20{break;}}drop(state);
            return Ok(json_response(200,super::skills_catalog::discover(&home(),&projects,&repo.settings.config_directory().join("skills"))));
        }
        if path=="/api/app-chat/send" {return Ok(json_response(200,repo.app_chat.start(repo,&payload)?));}
        if path=="/api/app-chat/new" {return Ok(json_response(200,repo.app_chat.new_chat(repo)?));}
        if path=="/api/app-chat/cancel" {return Ok(json_response(200,repo.app_chat.cancel()));}
        if path=="/api/settings/preview" {return Ok(json_response(200,repo.settings.preview(&payload,false)?));}
        if path=="/api/settings" {return Ok(json_response(200,repo.settings.apply(&payload,false)?));}
        if path=="/api/quick-actions/run" {return Ok(json_response(200,repo.quick_actions.start(repo,&payload)?));}
        let mut extra = json!({});
        if path.starts_with("/api/sharing/") {
            return Ok(json_response(200, super::sharing::action(repo, &path, &payload)?));
        }
        if path == "/api/session/links" {
            if let Some(context) = payload.get("context") {
                return Ok(json_response(200,super::session_context::replace_receipt(repo,text(&payload,"id"),context)?));
            }
            return Ok(json_response(200,super::session_context::read(repo,text(&payload,"id"))?));
        }
        if path == "/api/session/review-draft" {
            return Ok(json_response(200, super::reviews::action(repo,&payload)?));
        }
        if path == "/api/session/ci-investigation" {
            return Ok(json_response(200, super::session_ci::investigation(repo,&payload)?));
        }
        if path == "/api/session/ci-refresh" {
            return Ok(json_response(200, super::session_ci::refresh(repo,text(&payload,"id"))?));
        }
        if path == "/api/session/git-file" {
            return Ok(json_response(200, super::git_diff::file_for_session(repo,text(&payload,"id"),text(&payload,"path"),text(&payload,"cwd"))?));
        }
        if path == "/api/session/git-diff" {
            return Ok(json_response(200, super::git_diff::for_session(repo, text(&payload, "id"), text(&payload,"cwd"))?));
        }
        if path.starts_with("/api/terminal/") {
            let id = sid(text(&payload, "id"))?;
            let reply = match path.as_str() {
                "/api/terminal/poll" => repo.embedded.poll(
                    &id,
                    payload["cursor"].as_u64().unwrap_or(0),
                    text(&payload, "generation"),
                )?,
                "/api/terminal/close" => {
                    repo.embedded.close(&id, text(&payload, "generation"))?;
                    json!({"message":"Terminal closed. The saved conversation remains in History."})
                }
                "/api/terminal/input" => {
                    let data = payload["data"]
                        .as_str()
                        .ok_or(Error::new("Expected terminal input."))?;
                    repo.embedded.input(&id, data)?;
                    json!({})
                }
                "/api/terminal/resize" => {
                    let cols = payload["cols"]
                        .as_u64()
                        .filter(|n| *n <= 500)
                        .ok_or(Error::new("Invalid columns."))?
                        as u16;
                    let rows = payload["rows"]
                        .as_u64()
                        .filter(|n| *n <= 200)
                        .ok_or(Error::new("Invalid rows."))? as u16;
                    repo.embedded.resize(&id, cols, rows)?;
                    json!({})
                }
                _ => return Err(Error::new("Terminal operation unavailable.")),
            };
            return Ok(json_response(200, reply));
        }
        let saved = match path.as_str() {
            "/api/agent/register" => {
                let record = workflows::register(repo, &payload)?;
                return Ok(json_response(
                    200,
                    workflows::registration_reply(&record),
                ));
            }
            "/api/edit" => {
                repo.edit(&payload)?;
                None
            }
            "/api/github/pr" => {super::github::action(repo,&payload)?;None}
            "/api/github/reviews/seen" => {super::github::seen(repo,&payload)?;None}
            "/api/session/prepare" => Some(workflows::prepare(repo, &payload)?),
            "/api/session/from-view" => Some(super::view_context::prepare(repo, &payload)?),
            "/api/session/quick" => Some(workflows::quick(repo, &payload, false)?),
            "/api/session/pr-review" => Some(workflows::quick(repo, &payload, true)?),
            "/api/session/review" => Some(workflows::review(repo, &payload)?),
            "/api/session/newsletter" => Some(workflows::newsletter(repo, &payload)?),
            "/api/skills/run" => Some(super::scheduler::execute(
                repo,
                text(&payload, "presetId"),
                false,
                None,
            )?),
            "/api/skills/preset" => {
                super::scheduler::save_preset(repo, &payload)?;
                None
            }
            "/api/skills/schedule" => {
                super::scheduler::save_schedule(repo, &payload)?;
                None
            }
            "/api/skills/read" => {
                repo.transaction(|s| {
                    s["skillsReadAt"] = json!(now());
                    Ok(())
                })?;
                None
            }
            "/api/session/recover" => Some(workflows::recover(repo, &payload)?),
            "/api/session/open" => {
                let fields=payload.as_object().ok_or(Error::new("Open arguments must be an object."))?;
                if fields.keys().any(|key|!["id","target"].contains(&key.as_str())){return Err(Error::new("Open accepts only a saved chat ID and destination intent."));}
                let mut args=json!({"recordId":payload["id"]});if let Some(target)=payload.get("target"){args["target"]=target.clone();}
                let receipt=workflows::open_chat(repo,&args)?;
                extra["message"]=receipt["message"].clone();
                if receipt["status"]=="accepted"&&receipt["target"]=="embedded"{extra["embedded"]=json!(true);}
                if receipt["status"]=="preview"{extra["preview"]=json!(true);}
                extra["openReceipt"]=receipt;
                None
            }
            "/api/session/context" => {
                return Ok(json_response(
                    200,
                    json!({"context":workflows::context(repo,text(&payload,"id"))?}),
                ))
            }
            "/api/gcp-auth/start" => return Ok(json_response(200, repo.gcp_auth.start(repo.demo)?)),
            "/api/gcp-auth/cancel" => return Ok(json_response(200, repo.gcp_auth.cancel())),
            "/api/sync" => {
                let receipt = repo.refresh_sources(payload["force"] == true)?;
                extra["message"] = receipt["message"].clone();
                extra["refreshReceipt"] = receipt;
                None
            }
            "/api/sentry/environment" => {
                let environment = checked(&payload, "environment", 100, true)?;
                if !environment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
                {
                    return Err(Error::new("Use a single environment name."));
                }
                if !repo.demo && !repo.providers.sentry_configured() {
                    return Err(Error::new("Configure Sentry first."));
                }
                let should_start = claim_sentry_environment(repo,&environment)?;
                if should_start {
                    let repository = repo.clone();
                    std::thread::spawn(move || {
                        let result = repository.providers.fetch("sentry", Some(&environment));
                        let _ = repository.transaction(|s| {
                            let cache = &mut s["sentryEnvironments"][&environment];
                            match result {
                                Ok(items) => *cache = json!({"running":false,"itemIds":items.iter().map(|x|x["id"].clone()).collect::<Vec<_>>(),"lastSuccess":now(),"nextAttempt":now()+300.0,"error":null,"failures":0}),
                                Err(e) => {
                                    let failures = cache["failures"].as_u64().unwrap_or(0)+1;
                                    cache["running"] = json!(false);
                                    cache["error"] = json!(e.message);
                                    cache["failures"] = json!(failures);
                                    cache["nextAttempt"] = json!(now()+(300.0*2.0f64.powi((failures-1).min(4) as i32)).min(3600.0).max(e.retry));
                                }
                            }
                            Ok(())
                        });
                    });
                }
                None
            }
            _ => return Ok(json_response(404, json!({"error":"Not found."}))),
        };
        let mut state = repo.snapshot(false);
        if let Some(record) = saved {
            state["savedSessionId"] = record["id"].clone();
        }
        for (k, v) in extra.as_object().unwrap() {
            state[k] = v.clone();
        }
        Ok(json_response(200, state))
    })();
    let response = match result {
        Ok(r) => r,
        Err(e) => {
            let mut data = e.data;
            if !data.is_object() {
                data = json!({});
            }
            data["error"] = json!(e.message);
            json_response(e.status, data)
        }
    };
    let _ = req.respond(response);
}
fn ascii_json_string(value: &str) -> String {
    let encoded = serde_json::to_string(value).unwrap();
    let mut result = String::new();
    for c in encoded.chars() {
        if c as u32 > 127 {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                result.push_str(&format!("\\u{unit:04x}"));
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
    for entry in entries.filter_map(|e| e.ok()).take(100) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".result.json")||name.ends_with(".tmp"){if std::fs::symlink_metadata(&path).ok().and_then(|m|m.modified().ok()).and_then(|t|t.elapsed().ok()).is_some_and(|age|age.as_secs()>300){let _=std::fs::remove_file(&path);}continue;}
        if !name.ends_with(".request.json") {
            continue;
        }
        let mut reply_needed=true;
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
            reply_needed=data["reply"]!=false;
            let record = workflows::register(repo, &data["payload"])?;
            Ok(workflows::registration_reply(&record))
        })();
        let reply = match result {
            Ok(v) => v,
            Err(e) => json!({"error":e.message}),
        };
        let result_path = inbox.join(name.replace(".request.json", ".result.json"));
        if reply_needed&&!result_path.exists() {
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
            let record = arr(&repo.state.lock().unwrap()["sessions"])
                .into_iter()
                .find(|s| s["id"] == id)
                .ok_or(Error::new("Preview terminal record unavailable."))?;
            repo.embedded.open_demo(&record)?;
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
        let _ = sync_repo.schedule(false);
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
                json!({"error":"Too many local requests. Retry shortly."}),
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
mod environment_admission_tests{
 use super::*;
 #[test]fn sentry_environment_paused_claim_rejects_without_cache_cooldown_or_provider_admission(){
  let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();repo.transaction(|state|{state["sources"]["sentry"]["disabled"]=json!(true);state["sentryEnvironments"]=json!({"qa":{"lastSuccess":42,"itemIds":["cached"],"extension":"keep"}});Ok(())}).unwrap();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let cooldown=repo.cooldown.lock().unwrap().clone();let mut starts=0;let claim=claim_sentry_environment(&repo,"new-environment");if matches!(claim,Ok(true)){starts+=1;}assert!(claim.unwrap_err().message.contains("paused"));assert_eq!(starts,0);assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert_eq!(*repo.cooldown.lock().unwrap(),cooldown);
 }
}
