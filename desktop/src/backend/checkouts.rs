//! A chat's local checkouts: which folders belong to it, their Git metadata and their versions.
use super::agent_bridge::WorktreeReport;
use super::{
    arr, directory, expand, git, json, now, safe_url, sessions, text, Error, PathBuf, Repository,
    Result, Value,
};
use item_folders::Strength;
use std::{
    fs,
    path::{Component, Path},
};

pub(super) mod item_folders;

const MAX_FILES: usize = 500;
const MAX_LINKED: usize = 64;

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
/// A checkout to link to a chat, with the chat's work items done in it.
pub(super) struct Link {
    /// From `metadata`.
    pub metadata: Value,
    pub item_ids: Vec<Value>,
    /// Makes the checkout the chat's working directory.
    pub primary: bool,
}
/// What remembering a folder does to a different folder already remembered for the item.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Remember {
    Replace,
    Preserve,
}
/// Link a checkout to `chat`, or add items to the linked one at the same path, and
/// remember the checkout as the explicit folder of those items.
pub(super) fn link(
    state: &mut Value,
    chat: &str,
    link: &Link,
    existing: Remember,
) -> Result<Value> {
    let (metadata, ids) = (&link.metadata, &link.item_ids);
    let record = sessions::get(state, chat)?;
    let linked = arr(&record["items"]);
    if ids
        .iter()
        .any(|id| !linked.iter().any(|item| item["id"] == *id))
    {
        return Err(Error::new(
            "Register these work items with this session before linking their worktree.",
        ));
    }
    if record.get("worktrees").is_some() && !record["worktrees"].is_array() {
        return Err(Error::new("Invalid stored worktrees: expected a list."));
    }
    let mut trees = arr(&record["worktrees"]);
    let old = trees
        .iter()
        .find(|x| x["path"] == metadata["path"])
        .cloned()
        .unwrap_or(Value::Null);
    let mut item_ids = arr(&old["itemIds"]);
    for id in ids {
        if !item_ids.contains(id) {
            item_ids.push(id.clone());
        }
    }
    let mut entry = metadata.clone();
    entry["itemIds"] = json!(item_ids);
    entry["primary"] = json!(link.primary || old["primary"] == true);
    if link.primary {
        for tree in &mut trees {
            tree["primary"] = json!(false);
        }
    }
    trees.retain(|x| x["path"] != metadata["path"]);
    if trees.len() >= MAX_LINKED {
        return Err(Error::new("A session can link at most 64 worktrees."));
    }
    trees.push(entry);
    let row = sessions::get_mut(state, chat)?;
    row["worktrees"] = json!(trees);
    if link.primary {
        row["cwd"] = metadata["path"].clone();
    }
    row["updatedAt"] = json!(now());
    for id in ids {
        let item = id
            .as_str()
            .ok_or_else(|| Error::new("Invalid work item ID."))?;
        item_folders::remember(
            state,
            item,
            text(metadata, "path"),
            Strength::Explicit,
            existing,
        );
    }
    Ok(sessions::get(state, chat)?.clone())
}
/// Replace every checkout linked to `chat` with `links`.
pub(super) fn replace_all(state: &mut Value, chat: &str, links: &[Link]) -> Result<()> {
    sessions::get_mut(state, chat)?["worktrees"] = json!([]);
    for entry in links {
        link(state, chat, entry, Remember::Replace)?;
    }
    Ok(())
}
/// Drop every work item but `keep` from the checkouts linked to `chat`.
pub(super) fn unlink_items(state: &mut Value, chat: &str, keep: &[Value]) -> Result<()> {
    let record = sessions::get_mut(state, chat)?;
    for tree in record["worktrees"].as_array_mut().into_iter().flatten() {
        tree["itemIds"] = json!(arr(&tree["itemIds"])
            .into_iter()
            .filter(|x| keep.contains(x))
            .collect::<Vec<_>>());
    }
    Ok(())
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

/// Link a checkout an agent reports to its chat and to the chat's work items it names.
pub(super) fn link_reported(repo: &Repository, report: &WorktreeReport) -> Result<Value> {
    let path = directory(&report.path)?;
    let metadata = metadata(&path).ok_or_else(|| {
        Error::new("Choose an existing Git repository or worktree with a commit.")
    })?;
    let record = sessions::get(&repo.state.lock().unwrap(), &report.id)?.clone();
    if report.item_ids.len() + report.urls.len() > 100 {
        return Err(Error::new("Link at most 100 work items at once."));
    }
    let mut ids = report
        .item_ids
        .iter()
        .map(|id| json!(id))
        .collect::<Vec<_>>();
    for url in &report.urls {
        safe_url(url)?;
        let item = arr(&record["items"])
            .into_iter()
            .find(|x| x["url"] == *url)
            .ok_or_else(|| {
                Error::new("Register this URL with the session before linking its worktree.")
            })?;
        if !ids.contains(&item["id"]) {
            ids.push(item["id"].clone());
        }
    }
    if ids.is_empty() && report.urls.is_empty() {
        ids = arr(&record["items"])
            .iter()
            .map(|x| x["id"].clone())
            .collect();
    }
    let link = Link {
        metadata,
        item_ids: ids,
        primary: report.primary,
    };
    repo.transaction(|s| self::link(s, &report.id, &link, Remember::Replace))
}

#[cfg(test)]
mod tests;
