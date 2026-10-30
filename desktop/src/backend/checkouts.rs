//! A chat's local checkouts: which folders belong to it, their Git metadata and their versions.
use super::{directory, expand, git, json, now, text, Error, PathBuf, Result, Value};
use std::{
    fs,
    path::{Component, Path},
};

pub(super) mod item_folders;

const MAX_FILES: usize = 500;

/// The chat's working directory, then its linked worktrees, without blanks or repeats.
pub(super) fn of(chat: &Value) -> Vec<&str> {
    let trees = chat["worktrees"].as_array().into_iter().flatten();
    let mut paths = vec![];
    for path in std::iter::once(text(chat, "cwd")).chain(trees.map(|tree| text(tree, "path"))) {
        if !path.is_empty() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}
/// The stored checkout of `chat` that is the same folder as `path`.
///
/// Both sides resolve `~`, `..` and symlinks first, so a symlink out of a checkout never matches.
/// Callers act on the returned stored path, not on the requested spelling.
pub(super) fn find<'a>(chat: &'a Value, path: &str) -> Option<&'a str> {
    let folder = resolve(path)?;
    of(chat)
        .into_iter()
        .find(|checkout| resolve(checkout).as_ref() == Some(&folder))
}
fn resolve(path: &str) -> Option<PathBuf> {
    let path = expand(path);
    if !path.is_absolute() {
        return None;
    }
    path.canonicalize().ok().filter(|path| path.is_dir())
}
/// What remembering a folder does to a different folder already remembered for the item.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Remember {
    Replace,
    Preserve,
}
/// Git metadata is read only during explicit registration, never during UI reads.
pub(super) fn metadata(cwd: &str) -> Option<Value> {
    let output = git::run(
        Path::new(cwd),
        &[
            "rev-parse",
            "--path-format=absolute",
            "--show-toplevel",
            "--git-common-dir",
            "--abbrev-ref",
            "HEAD",
        ],
    )
    .ok()?;
    let mut lines = output.lines();
    let path = directory(lines.next()?).ok()?;
    let common = PathBuf::from(lines.next()?);
    let branch = lines.next()?.to_owned();
    let repo_path = if common.file_name()?.to_str()? == ".git" {
        directory(common.parent()?.to_str()?).ok()?
    } else {
        path.clone()
    };
    Some(
        json!({"path":path,"repoPath":repo_path,"branch":branch,"isWorktree":path!=repo_path,"updatedAt":now()}),
    )
}
pub(super) struct Worktree {
    pub path: PathBuf,
    /// Short branch name; none for a detached or bare worktree.
    pub branch: Option<String>,
}
/// Every worktree of the repository that contains `root`, as Git lists them.
pub(super) fn worktrees(root: &Path) -> Result<Vec<Worktree>> {
    let list = git::run(root, &["worktree", "list", "--porcelain"])?;
    let mut trees: Vec<Worktree> = vec![];
    for line in list.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            trees.push(Worktree {
                path: PathBuf::from(path),
                branch: None,
            });
        } else if let (Some(branch), Some(tree)) =
            (line.strip_prefix("branch refs/heads/"), trees.last_mut())
        {
            tree.branch = Some(branch.to_owned());
        }
    }
    Ok(trees)
}
/// The main branch that work in a checkout is compared against.
pub(super) struct Base {
    /// `origin/main` or `main`.
    pub name: &'static str,
    pub commit: String,
}
/// The remote main when it is fetched, else the local main.
pub(super) fn base(root: &Path) -> Result<Base> {
    for (name, reference) in [
        ("origin/main", "refs/remotes/origin/main"),
        ("main", "refs/heads/main"),
    ] {
        let spec = format!("{reference}^{{commit}}");
        if let Ok(commit) = git::run(root, &["rev-parse", "--verify", &spec]) {
            return Ok(Base {
                name,
                commit: commit.trim().to_owned(),
            });
        }
    }
    Err(Error::new(
        "No main branch is available locally. Fetch main, then retry.",
    ))
}
/// The version of each checkout's Git root. Null marks a root whose version cannot be read.
pub(super) fn versions(chat: &Value) -> Value {
    let mut checkouts = of(chat);
    checkouts.sort_unstable();
    let mut result = json!({});
    for checkout in checkouts {
        match version(checkout) {
            Ok((root, version)) => result[root] = json!(version),
            Err(_) => result[checkout] = Value::Null,
        }
    }
    result
}
fn version(checkout: &str) -> Result<(String, String)> {
    let identity = git::run(
        Path::new(checkout),
        &["rev-parse", "--show-toplevel", "HEAD"],
    )?;
    let root = identity
        .lines()
        .next()
        .ok_or_else(|| Error::new("Git root unavailable."))?
        .to_owned();
    let status = git::run(
        Path::new(&root),
        &["status", "--porcelain=v1", "--untracked-files=all", "-z"],
    )?;
    let base = base(Path::new(&root))?;
    // Saved versions hash the base commit as the line `rev-parse` prints.
    let mut stamp = identity + &base.commit + "\n" + &status;
    let mut entries = status.split('\0').filter(|s| !s.is_empty());
    let mut count = 0;
    while let Some(entry) = entries.next() {
        count += 1;
        if count > MAX_FILES {
            return Err(Error::new("Too many files to verify review freshness."));
        }
        let path = entry
            .get(3..)
            .ok_or_else(|| Error::new("Invalid Git status."))?;
        match fs::symlink_metadata(Path::new(&root).join(valid_path(path)?)) {
            Ok(meta) => stamp.push_str(&metadata_stamp(&meta)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => stamp.push_str("deleted"),
            Err(error) => return Err(error.into()),
        }
        if entry[..2].contains(['R', 'C']) {
            entries.next();
        }
    }
    Ok((root, context_kit::sha256(stamp.as_bytes())))
}
/// The version of one file in a checkout; none when it cannot be read.
pub(super) fn file_version(root: &str, path: &str) -> Option<String> {
    let target = Path::new(root).join(valid_path(path).ok()?);
    match fs::symlink_metadata(target) {
        Ok(m) => Some(metadata_stamp(&m)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some("deleted".into()),
        Err(_) => None,
    }
}
fn metadata_stamp(m: &fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!(
        "{}:{}:{}:{}:{}:{}",
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec()
    )
}
/// A path relative to a checkout root, as Git prints it.
pub(super) fn valid_path(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Error::new("Git returned an invalid file path."));
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
