//! Versioned JSON configuration files that clients replace by compare-and-swap.
use super::{json, Error, Result, Value};
use serde::Serialize;
use std::{io::Read, marker::PhantomData, path::PathBuf, sync::Mutex};

// Configuration writes are rare and small, so one lock serves every document.
static WRITE_LOCK: Mutex<()> = Mutex::new(());
// Clients hold revisions as JavaScript numbers.
const MAX_REVISION: u64 = 9_007_199_254_740_991;

/// A stored document format. Validation, defaults and migrations stay with its owner.
pub trait Format: Serialize + Sized {
    /// Names the document in errors, such as "saved views configuration".
    const NAME: &'static str;
    const MAX_BYTES: usize;
    /// Tells the client how to recover from a stale version.
    const CONFLICT: &'static str;
    /// Parse and validate stored bytes, migrating older layouts.
    fn parse(bytes: &[u8]) -> Result<Self>;
    fn revision(&self) -> u64;
}

/// The token a client reads and sends back to replace a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub revision: u64,
    /// SHA-256 of the stored bytes, so edits made outside the app also conflict.
    pub content_hash: String,
}
impl Version {
    /// Take `revision` and `contentHash` from a request. A missing or malformed token never matches.
    pub fn sent(payload: &Value) -> Self {
        match (
            payload["revision"].as_u64(),
            payload["contentHash"].as_str(),
        ) {
            (Some(revision), Some(hash)) => Self {
                revision,
                content_hash: hash.to_owned(),
            },
            _ => Self {
                revision: 0,
                content_hash: String::new(),
            },
        }
    }
}

pub struct Document<T> {
    path: PathBuf,
    missing: Option<Vec<u8>>,
    format: PhantomData<fn() -> T>,
}
impl<T: Format> Document<T> {
    pub const fn new(path: PathBuf) -> Self {
        Self {
            path,
            missing: None,
            format: PhantomData,
        }
    }
    pub fn read(&self) -> Result<(T, Version)> {
        let bytes = self.bytes()?;
        let document = T::parse(&bytes)?;
        let version = version(&document, &bytes)?;
        Ok((document, version))
    }
    /// Read the document only while it is still at `expected`.
    pub fn read_at(&self, expected: &Version) -> Result<T> {
        let (document, version) = self.read()?;
        if version != *expected {
            let mut error = Error::new(T::CONFLICT);
            error.status = 409;
            return Err(error);
        }
        Ok(document)
    }
    /// Store `next` at the following revision while the document is still at `expected`.
    pub fn replace(&self, expected: &Version, next: T) -> Result<(T, Version)> {
        let _guard = WRITE_LOCK.lock().map_err(|_| Error::io())?;
        self.read_at(expected)?;
        let mut value = serde_json::to_value(next)?;
        // `read_at` bounds the current revision, and `version` refuses a next one past the limit.
        value["revision"] = json!(expected.revision + 1);
        let bytes = serde_json::to_vec_pretty(&value)?;
        if bytes.len() > T::MAX_BYTES {
            return Err(too_large::<T>());
        }
        let document = T::parse(&bytes)?;
        let version = version(&document, &bytes)?;
        super::http::private_write(&self.path, &bytes)?;
        Ok((document, version))
    }
    fn bytes(&self) -> Result<Vec<u8>> {
        let unreadable = || Error::new(format!("Cannot read {}.", T::NAME));
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return self.missing.clone().ok_or_else(unreadable)
            }
            Err(_) => return Err(unreadable()),
        };
        let mut bytes = Vec::new();
        file.take(T::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| unreadable())?;
        if bytes.len() > T::MAX_BYTES {
            return Err(too_large::<T>());
        }
        Ok(bytes)
    }
}
fn version<T: Format>(document: &T, bytes: &[u8]) -> Result<Version> {
    let revision = document.revision();
    if revision > MAX_REVISION {
        return Err(Error::new(format!(
            "The {} revision is exhausted.",
            T::NAME
        )));
    }
    Ok(Version {
        revision,
        content_hash: context_kit::sha256(bytes),
    })
}
fn too_large<T: Format>() -> Error {
    Error::new(format!(
        "The {} exceeds {} KiB.",
        T::NAME,
        T::MAX_BYTES / 1024
    ))
}

#[cfg(test)]
mod tests;
