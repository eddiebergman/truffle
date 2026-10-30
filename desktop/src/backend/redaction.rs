//! Output policy: remove credentials from values that leave the service.
use super::{Repository, Value};

pub(super) fn sensitive_environment_values() -> impl Iterator<Item = String> {
    std::env::vars()
        .filter(|(key, value)| {
            let key = key.to_ascii_uppercase();
            (key.contains("TOKEN")
                || key.contains("SECRET")
                || key.contains("PASSWORD")
                || key.ends_with("API_KEY"))
                && value.len() >= 8
        })
        .map(|(_, value)| value)
}
pub(super) fn stream_redactor(repo: &Repository) -> super::providers::StreamRedactor {
    repo.providers
        .stream_redactor()
        .with_values(sensitive_environment_values())
}
pub(super) fn redact_text(repo: &Repository, text: &mut String) {
    repo.providers.redact_text(text);
    for value in sensitive_environment_values() {
        *text = text.replace(&value, "[redacted]");
    }
}
pub(super) fn redact(repo: &Repository, v: &mut Value) {
    match v {
        Value::Object(map) => {
            map.retain(|k, _| {
                !k.contains('\0')
                    && !matches!(
                        k.to_ascii_lowercase().as_str(),
                        "token"
                            | "authorization"
                            | "password"
                            | "secret"
                            | "apikey"
                            | "api_key"
                            | "cookie"
                    )
            });
            for value in map.values_mut() {
                redact(repo, value);
            }
        }
        Value::Array(rows) => {
            for row in rows {
                redact(repo, row);
            }
        }
        Value::String(text) => redact_text(repo, text),
        _ => {}
    }
}
#[cfg(test)]
mod tests;
