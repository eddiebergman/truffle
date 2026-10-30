use super::*;
use serde::Deserialize;
use std::os::unix::fs::PermissionsExt;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Note {
    version: u64,
    revision: u64,
    text: String,
}
impl Format for Note {
    const NAME: &'static str = "note";
    const MAX_BYTES: usize = 1024;
    const CONFLICT: &'static str = "The note changed.";
    fn parse(bytes: &[u8]) -> Result<Self> {
        let note: Self = serde_json::from_slice(bytes)?;
        if note.version != 1 {
            return Err(Error::new("Unsupported note version."));
        }
        Ok(note)
    }
    fn revision(&self) -> u64 {
        self.revision
    }
}
fn note(text: &str) -> Note {
    Note {
        version: 1,
        revision: 0,
        text: text.into(),
    }
}
fn stored(text: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("note.json");
    std::fs::write(&path, serde_json::to_vec(&note(text)).unwrap()).unwrap();
    (temp, path)
}

#[test]
fn replace_advances_the_revision_and_returns_the_stored_version() {
    let (_temp, path) = stored("first");
    let document = Document::<Note>::new(path.clone());
    let (_, version) = document.read().unwrap();
    let (written, next) = document.replace(&version, note("second")).unwrap();
    assert_eq!(written.revision, 1);
    assert_eq!(next.revision, 1);
    assert_eq!(
        next.content_hash,
        context_kit::sha256(&std::fs::read(&path).unwrap())
    );
    let (read, reread) = document.read().unwrap();
    assert_eq!(read.text, "second");
    assert_eq!(reread, next);
}

#[test]
fn a_stale_or_missing_version_is_refused_without_writing() {
    let (_temp, path) = stored("first");
    let document = Document::<Note>::new(path.clone());
    let (_, version) = document.read().unwrap();
    let (_, current) = document.replace(&version, note("second")).unwrap();
    // Same revision, other bytes: an edit made outside the app.
    let mut external = note("external");
    external.revision = 1;
    std::fs::write(&path, serde_json::to_vec(&external).unwrap()).unwrap();
    let saved = std::fs::read(&path).unwrap();
    for stale in [version, current, Version::sent(&json!({"revision": 1}))] {
        let error = document.replace(&stale, note("lost")).unwrap_err();
        assert_eq!(
            (error.status, error.message.as_str()),
            (409, "The note changed.")
        );
        assert_eq!(document.read_at(&stale).unwrap_err().status, 409);
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }
}

#[test]
fn concurrent_replaces_of_one_version_let_exactly_one_win() {
    for _ in 0..20 {
        let (_temp, path) = stored("first");
        let (_, version) = Document::<Note>::new(path.clone()).read().unwrap();
        let barrier = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let writers = ["left", "right"].map(|text| {
                let (path, version, barrier) = (path.clone(), &version, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    Document::<Note>::new(path).replace(version, note(text))
                })
            });
            writers.map(|writer| writer.join().unwrap())
        });
        let winners = results
            .iter()
            .filter_map(|result| result.as_ref().ok())
            .collect::<Vec<_>>();
        assert_eq!(winners.len(), 1);
        assert!(results
            .iter()
            .any(|result| result.as_ref().is_err_and(|error| error.status == 409)));
        let (stored, _) = Document::<Note>::new(path).read().unwrap();
        assert_eq!(stored.text, winners[0].0.text);
    }
}

#[test]
fn oversized_files_and_replacements_are_refused() {
    let (_temp, path) = stored("first");
    let document = Document::<Note>::new(path.clone());
    let (_, version) = document.read().unwrap();
    let error = document
        .replace(&version, note(&"x".repeat(Note::MAX_BYTES)))
        .unwrap_err();
    assert_eq!(error.message, "The note exceeds 1 KiB.");
    assert_eq!(document.read().unwrap().1, version);
    std::fs::write(&path, serde_json::to_vec(&note(&"x".repeat(1000))).unwrap()).unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > 1024);
    assert_eq!(
        document.read().unwrap_err().message,
        "The note exceeds 1 KiB."
    );
}

#[test]
fn a_failed_write_leaves_the_old_file_intact() {
    let (temp, path) = stored("first");
    let document = Document::<Note>::new(path.clone());
    let (_, version) = document.read().unwrap();
    let saved = std::fs::read(&path).unwrap();
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = document.replace(&version, note("second"));
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn replacements_must_parse_and_keep_revisions_safe_for_clients() {
    let (_temp, path) = stored("first");
    let document = Document::<Note>::new(path.clone());
    let (_, version) = document.read().unwrap();
    let mut unsupported = note("second");
    unsupported.version = 2;
    assert!(document.replace(&version, unsupported).is_err());
    assert_eq!(document.read().unwrap().1, version);
    let mut last = note("last");
    last.revision = MAX_REVISION;
    std::fs::write(&path, serde_json::to_vec(&last).unwrap()).unwrap();
    let (_, version) = document.read().unwrap();
    assert!(document.replace(&version, note("next")).is_err());
    last.revision = MAX_REVISION + 1;
    std::fs::write(&path, serde_json::to_vec(&last).unwrap()).unwrap();
    assert!(document.read().is_err());
}
