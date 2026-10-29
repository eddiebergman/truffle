pub mod agent_cli;
mod agent_access;
mod local_files;
pub mod chat_terminal;
pub mod chat_preparation;
pub mod chat_edit;
pub mod commands;
pub mod connections;
pub mod onboarding;
pub mod guidance;
pub mod page_settings;
pub mod feedback;
pub mod chat_search;
pub mod work_search;
pub mod work_links;
pub mod views;
pub mod config;
pub mod config_status;
pub mod skills_catalog;
pub mod app_chat;
pub mod presentation;
pub mod settings;
pub mod quick_actions;
pub mod assets { include!(concat!(env!("OUT_DIR"),"/assets.rs")); }
pub mod artifacts;
pub mod domain;
pub mod projects;
pub mod changes;
pub mod embedded;
pub mod http;
pub mod gcp_auth;
pub mod github;
pub mod git_diff;
pub mod providers;
pub mod repository;
pub mod resources;
pub mod reviews;
pub mod session_context;
pub mod session_ci;
pub mod session_dispatch;
pub mod terminal_daemon;
pub mod scheduler;
pub mod skill_presets;
pub mod sharing;
pub mod terminal;
pub mod workflows;
pub mod view_context;
pub use repository::Repository;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Debug, Clone)]
pub struct Error {
    pub message: String,
    pub status: u16,
    pub data: Value,
    pub retry: f64,
}
impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: 400,
            data: json!({}),
            retry: 0.0,
        }
    }
    pub fn io() -> Self {
        Self {
            message: "Local operation failed. Check file permissions and available disk space."
                .into(),
            status: 500,
            data: json!({}),
            retry: 0.0,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::io()
    }
}
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::new("Invalid JSON data.")
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
pub fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
pub fn arr(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}
pub fn expand(path: &str) -> PathBuf {
    if path == "~" {
        home()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(path)
    }
}
pub fn directory(path: &str) -> Result<String> {
    let p = expand(path);
    if path.is_empty() || !p.is_dir() || path.chars().any(char::is_control) {
        return Err(Error::new(
            "Choose an existing repository or worktree folder.",
        ));
    }
    Ok(p.canonicalize()?.to_string_lossy().into())
}
pub fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn sid(value: &str) -> Result<String> {
    uuid::Uuid::parse_str(value)
        .map(|x| x.to_string())
        .map_err(|_| Error::new("Enter the exact conversation UUID."))
}
pub fn checked(v: &Value, key: &str, max: usize, required: bool) -> Result<String> {
    let s = v[key].as_str().unwrap_or("");
    if s.chars().count() > max || s.contains('\0') || (required && s.trim().is_empty()) {
        return Err(Error::new(format!("Invalid {key}.")));
    }
    Ok(s.trim().into())
}
pub fn safe_url(value: &str) -> Result<String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    let u = reqwest::Url::parse(value).map_err(|_| Error::new("Use an HTTP or HTTPS URL."))?;
    if !["http", "https"].contains(&u.scheme())
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || value.chars().any(char::is_control)
    {
        return Err(Error::new("Use an HTTP or HTTPS URL without credentials."));
    }
    Ok(value.into())
}
#[cfg(test)]
mod tests;
