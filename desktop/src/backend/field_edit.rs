//! Record edits that clients make field by field with compare-and-swap tokens.
use super::{json, redaction, Error, Repository, Result, Value};
use serde_json::Map;
use std::marker::PhantomData;

/// A record kind with editable fields. Storage and writes stay with its owner.
pub trait Record {
    /// Names the record in errors, such as "project".
    const NAME: &'static str;
    const FIELDS: &'static [&'static str];
    fn id(&self) -> &str;
    /// The stored data that the token of `field` guards.
    fn projection(&self, field: &str) -> Result<Value>;
    /// Validate one patched field and return the value to store.
    fn normalize(repo: &Repository, field: &str, value: &Value) -> Result<Value>;
}

/// Refuse a value that redaction or a control character would change on its way to a client.
pub fn exact(repo: &Repository, value: &str) -> Result<()> {
    let mut projected = value.to_owned();
    redaction::redact_text(repo, &mut projected);
    if projected != value || value.chars().any(char::is_control) {
        return Err(Error::new(
            "This identity or edit token cannot be safely projected.",
        ));
    }
    Ok(())
}

pub fn conflict(message: &str) -> Error {
    let mut error = Error::new(message);
    error.status = 409;
    error
}

fn token<R: Record>(record: &R, field: &str) -> Result<String> {
    Ok(context_kit::sha256(&serde_json::to_vec(
        &json!({"recordId":record.id(),"field":field,"value":record.projection(field)?}),
    )?))
}

/// The token of each editable field, keyed by field name.
pub fn tokens<R: Record>(repo: &Repository, record: &R) -> Result<Value> {
    exact(repo, record.id())?;
    let mut tokens = json!({});
    for field in R::FIELDS {
        exact(repo, field)?;
        let token = token(record, field)?;
        exact(repo, &token)?;
        tokens[*field] = json!(token);
    }
    Ok(tokens)
}

/// A checked patch and the tokens the client captured for exactly its fields.
pub struct Edit<R> {
    fields: Map<String, Value>,
    expected: Map<String, Value>,
    record: PhantomData<fn() -> R>,
}
impl<R: Record> Edit<R> {
    pub fn new(repo: &Repository, patch: &Value, expected: &Value) -> Result<Self> {
        let mismatch = || {
            Error::new(format!(
                "Supply captured tokens for exactly the changed {} fields.",
                R::NAME
            ))
        };
        let patch = patch
            .as_object()
            .ok_or_else(|| Error::new(format!("Supply {} fields as an object.", R::NAME)))?;
        if patch
            .keys()
            .any(|field| !R::FIELDS.contains(&field.as_str()))
        {
            return Err(Error::new(format!("Unknown {} field.", R::NAME)));
        }
        let expected = expected.as_object().ok_or_else(mismatch)?;
        if patch.is_empty()
            || patch.len() != expected.len()
            || patch.keys().any(|field| !expected.contains_key(field))
        {
            return Err(mismatch());
        }
        for token in expected.values() {
            let token = token
                .as_str()
                .filter(|token| {
                    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .ok_or_else(mismatch)?;
            exact(repo, token)?;
        }
        let mut fields = Map::new();
        for (field, value) in patch {
            fields.insert(field.clone(), R::normalize(repo, field, value)?);
        }
        Ok(Self {
            fields,
            expected: expected.clone(),
            record: PhantomData,
        })
    }
    /// The normalized value of a patched field.
    pub fn get(&self, field: &str) -> Option<&Value> {
        self.fields.get(field)
    }
    /// Refuse with 409 when a patched field changed since the client read its token.
    pub fn check(&self, record: &R) -> Result<()> {
        for field in self.fields.keys() {
            if self.expected[field] != token(record, field)? {
                return Err(conflict(&format!(
                    "A changed {} field was edited elsewhere. Read it again and reapply only your intended changes.",
                    R::NAME
                )));
            }
        }
        Ok(())
    }
    /// Check `record`, then pass each patched field to `write`. Return the fields that `write` changed.
    pub fn apply(
        &self,
        record: &R,
        mut write: impl FnMut(&str, &Value) -> Result<bool>,
    ) -> Result<Vec<String>> {
        self.check(record)?;
        let mut changed = vec![];
        for (field, value) in &self.fields {
            if write(field, value)? {
                changed.push(field.clone());
            }
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests;
