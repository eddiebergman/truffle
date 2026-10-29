//! UI-independent service port. No Tauri, DOM, layout, or persisted view state.
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde::Serialize;
use serde_json::{json, Value};
use std::{sync::Mutex, time::Duration};

const READ_ROUTES: &[&str] = &[
    "/api/config-status",
    "/api/onboarding",
    "/api/guidance",
    "/api/feedback",
    "/api/page-settings",
    "/api/views",
    "/api/commands",
    "/api/settings",
    "/api/themes",
    "/api/quick-actions",
    "/api/app-chat",
    "/api/app-chat/skill",
    "/api/state",
    "/api/agents",
    "/api/export",
    "/api/library",
    "/api/logs",
    "/api/gcp-auth",
    "/api/sharing",
];
const WRITE_ROUTES: &[&str] = &[
    "/api/connections/reload",
    "/api/onboarding",
    "/api/guidance",
    "/api/feedback",
    "/api/page-settings",
    "/api/views",
    "/api/commands/invoke",
    "/api/shell-terminal/open",
    "/api/shell-terminal/poll",
    "/api/shell-terminal/input",
    "/api/shell-terminal/resize",
    "/api/shell-terminal/close",
    "/api/settings",
    "/api/quick-actions/run",
    "/api/app-chat/send",
    "/api/app-chat/new",
    "/api/app-chat/cancel",
    "/api/skills/catalog",
    "/api/github/pr",
    "/api/github/reviews/seen",
    "/api/gcp-auth/start",
    "/api/gcp-auth/cancel",
    "/api/sharing/config",
    "/api/sharing/job",
    "/api/sharing/refresh",
    "/api/sharing/prepare",
    "/api/sharing/send",
    "/api/sharing/preview",
    "/api/sharing/import",
    "/api/sharing/dismiss",
    "/api/terminal/poll",
    "/api/terminal/input",
    "/api/terminal/resize",
    "/api/terminal/close",
    "/api/skills/run",
    "/api/skills/preset",
    "/api/skills/schedule",
    "/api/skills/read",
    "/api/edit",
    "/api/sync",
    "/api/session/prepare",
    "/api/session/quick",
    "/api/session/from-view",
    "/api/session/pr-review",
    "/api/session/review",
    "/api/session/newsletter",
    "/api/session/open",
    "/api/session/recover",
    "/api/session/context",
    "/api/session/git-diff",
    "/api/session/git-file",
    "/api/session/ci-refresh",
    "/api/session/ci-investigation",
    "/api/session/review-draft",
    "/api/session/links",
    "/api/sentry/environment",
];
pub const ARTIFACT_CSP: &str = "sandbox allow-popups allow-popups-to-escape-sandbox; default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors tauri://localhost http://tauri.localhost";

#[derive(Serialize)]
pub struct Reply {
    pub status: u16,
    pub data: Value,
}
pub struct Asset {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

pub trait DashboardService: Send + Sync {
    fn request(&self, path: &str, payload: Option<Value>) -> Result<Reply, String>;
    fn health(&self) -> Result<Value, String>;
    fn artifact(&self, id: &str) -> Result<Asset, String>;
}

pub struct LocalHttpService {
    base: Url,
    client: Client,
    csrf: Mutex<String>,
}
impl LocalHttpService {
    pub fn new(address: &str) -> Result<Self, String> {
        let base = Url::parse(address).map_err(|_| "Invalid service address.")?;
        if base.scheme() != "http"
            || base.host_str() != Some("127.0.0.1")
            || !base.username().is_empty()
            || base.password().is_some()
            || base.path() != "/"
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(
                "The service must use a plain 127.0.0.1 HTTP origin without credentials or a path."
                    .into(),
            );
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|_| "Cannot initialize the service adapter.")?;
        Ok(Self {
            base,
            client,
            csrf: Mutex::new(String::new()),
        })
    }
    pub fn origin(&self) -> String {
        self.base.origin().ascii_serialization()
    }
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.origin(), path)
    }
    pub fn artifact_url(&self, id: &str) -> Result<String, String> {
        validate_artifact_id(id)?;
        Ok(self.url(&format!("/artifacts/{id}")))
    }
    pub fn linear_url(&self, path: &str) -> Result<String, String> {
        let id = path
            .strip_prefix("/linear/")
            .ok_or("Invalid ticket link.")?;
        if id.is_empty()
            || !id
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err("Invalid ticket link.".into());
        }
        Ok(self.url(path))
    }
    fn token(&self) -> Result<String, String> {
        let token = self
            .csrf
            .lock()
            .map_err(|_| "Service authorization is unavailable.")?
            .clone();
        if !token.is_empty() {
            return Ok(token);
        }
        let _ = self.request("/api/state", None)?;
        Ok(self
            .csrf
            .lock()
            .map_err(|_| "Service authorization is unavailable.")?
            .clone())
    }
    fn capture_token(&self, body: &mut Value) {
        if let Some(token) = body.get("csrf").and_then(Value::as_str) {
            if let Ok(mut current) = self.csrf.lock() {
                *current = token.to_owned();
            }
        }
        // The web UI never receives the native client's service credential.
        if let Some(object) = body.as_object_mut() {
            object.remove("csrf");
        }
    }
}
impl DashboardService for LocalHttpService {
    fn request(&self, path: &str, payload: Option<Value>) -> Result<Reply, String> {
        let allowed = if payload.is_some() {
            WRITE_ROUTES
        } else {
            READ_ROUTES
        };
        if !allowed.contains(&path) {
            return Err("Unsupported dashboard operation.".into());
        }
        for attempt in 0..2 {
            let request = match &payload {
                Some(body) => self
                    .client
                    .post(self.url(path))
                    .header("Origin", self.origin())
                    .header("X-CSRF-Token", self.token()?)
                    .json(body),
                None => self.client.get(self.url(path)),
            };
            let response = request.send().map_err(|_| {
                "Cannot reach the local mydash service. Start ~/code/mydash/start.command in Terminal and retry."
            })?;
            let status = response.status().as_u16();
            let mut body: Value = response
                .json()
                .map_err(|_| "The local service returned an invalid response.")?;
            self.capture_token(&mut body);
            // A restarted service rejects the old CSRF token before performing any mutation.
            if status == 403 && payload.is_some() && attempt == 0 {
                let _ = self.request("/api/state", None)?;
                continue;
            }
            return Ok(Reply { status, data: body });
        }
        Err("Reload the local service connection and retry.".into())
    }
    fn health(&self) -> Result<Value, String> {
        let response = self
            .client
            .get(self.url("/api/health"))
            .timeout(Duration::from_secs(3))
            .send()
            .map_err(|_| "The mydash service is not reachable.")?;
        if !response.status().is_success() {
            return Err("The service needs an update. Restart mydash inside cmux.".into());
        }
        let body: Value = response.json().map_err(|_| "Invalid service identity.")?;
        if body["service"] != "mydash" || body["apiVersion"] != 1 {
            return Err("Unsupported local service. Expected mydash API version 1.".into());
        }
        Ok(body)
    }
    fn artifact(&self, id: &str) -> Result<Asset, String> {
        let response = self
            .client
            .get(self.artifact_url(id)?)
            .timeout(Duration::from_secs(10))
            .send()
            .map_err(|_| "Artifact unavailable.")?;
        if !response.status().is_success() {
            return Err("Artifact unavailable. Its file may have moved.".into());
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        if response.content_length().unwrap_or(0) > 10 * 1024 * 1024 {
            return Err("Artifact exceeds 10 MiB.".into());
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        response
            .take(10 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read artifact.")?;
        if bytes.len() > 10 * 1024 * 1024 {
            return Err("Artifact exceeds 10 MiB.".into());
        }
        Ok(Asset {
            bytes,
            content_type,
        })
    }
}
pub fn validate_artifact_id(id: &str) -> Result<(), String> {
    if id.len() == 24 && id.bytes().all(|x| x.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("Invalid artifact ID.".into())
    }
}
pub fn error_reply(message: String) -> Reply {
    Reply {
        status: 503,
        data: json!({"error":message}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_is_loopback_only() {
        for value in [
            "https://127.0.0.1:8765",
            "http://localhost:8765",
            "http://example.com",
            "http://127.0.0.1:8765/api",
            "http://user:secret@127.0.0.1",
            "http://127.0.0.1?token=x",
        ] {
            assert!(LocalHttpService::new(value).is_err(), "{value}");
        }
        assert_eq!(
            LocalHttpService::new("http://127.0.0.1:8765")
                .unwrap()
                .origin(),
            "http://127.0.0.1:8765"
        );
    }
    #[test]
    fn routes_cannot_escape_to_remote_or_credential_bridge() {
        let service = LocalHttpService::new("http://127.0.0.1:1").unwrap();
        for path in [
            "https://example.com",
            "//example.com",
            "/api/agent/register",
            "/api/state?x=secret",
            "/../.env",
        ] {
            assert!(service.request(path, None).is_err());
            assert!(service.request(path, Some(json!({}))).is_err());
        }
    }
    #[test]
    fn artifact_paths_are_opaque_ids() {
        assert!(validate_artifact_id("d82f1f3d3b8f1abc330d8205").is_ok());
        for path in [
            "../../.env",
            "file:///etc/passwd",
            "123",
            "d82f1f3d3b8f1abc330d8205?x=1",
        ] {
            assert!(validate_artifact_id(path).is_err());
        }
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    fn mock_server(replies: Vec<(u16, Value)>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    bytes.extend_from_slice(&buffer[..count]);
                    if bytes.windows(4).any(|x| x == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&bytes).to_string());
                let body = body.to_string();
                write!(stream,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        (address, handle)
    }
    #[test]
    fn service_authorization_is_rotated_after_restart_and_not_returned_to_ui() {
        let (address, server) = mock_server(vec![
            (200, json!({"csrf":"first","items":[]})),
            (403, json!({"error":"restart"})),
            (200, json!({"csrf":"second","items":[]})),
            (200, json!({"items":[]})),
        ]);
        let service = LocalHttpService::new(&address).unwrap();
        let state = service.request("/api/state", None).unwrap();
        assert!(state.data.get("csrf").is_none());
        assert_eq!(
            service
                .request(
                    "/api/edit",
                    Some(json!({"action":"save-note","title":"Test"}))
                )
                .unwrap()
                .status,
            200
        );
        let requests = server.join().unwrap();
        assert!(requests[1].to_lowercase().contains("x-csrf-token: first"));
        assert!(requests[3].to_lowercase().contains("x-csrf-token: second"));
        assert!(requests[3]
            .to_lowercase()
            .contains(&format!("origin: {address}")));
    }
    #[test]
    fn native_configuration_status_is_read_only_and_uses_the_local_service() {
        let document=json!({"version":1,"resources":{"settings":{"token":"a".repeat(64)}}});
        let (address,server)=mock_server(vec![(200,document.clone())]);
        let service=LocalHttpService::new(&address).unwrap();
        assert!(service.request("/api/config-status",Some(json!({}))).is_err());
        let response=service.request("/api/config-status",None).unwrap();
        assert_eq!(response.status,200);assert_eq!(response.data,document);
        let requests=server.join().unwrap();assert_eq!(requests.len(),1);assert!(requests[0].starts_with("GET /api/config-status HTTP/1.1"));
    }
    #[test]
    fn health_checks_identity_and_protocol() {
        let (address, server) = mock_server(vec![
            (200, json!({"service":"another-app","apiVersion":1})),
            (200, json!({"service":"mydash","apiVersion":1})),
        ]);
        let service = LocalHttpService::new(&address).unwrap();
        assert!(service.health().is_err());
        assert!(service.health().is_ok());
        server.join().unwrap();
    }
}
