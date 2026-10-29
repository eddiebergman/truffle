use super::*;
use serde::Serialize;
use std::{
    fs,
    io::Read,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Component, Path},
    time::{Duration, Instant},
};

const MAX_FILES: usize = 500;
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Default)]
pub struct Cache {
    entry: Option<(String, Instant, Value)>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileChange {
    path: String,
    additions: Option<u64>,
    deletions: Option<u64>,
    untracked: bool,
}
struct Comparison {
    root: PathBuf,
    base: String,
    base_ref: String,
    branch: String,
    files: Vec<FileChange>,
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let mut command = vec![
        "--no-optional-locks".into(),
        "-C".into(),
        root.to_string_lossy().into_owned(),
    ];
    command.extend(args.iter().map(|arg| arg.to_string()));
    terminal::run("git", &command, None, 10)
}
fn valid_path(path: &str) -> Result<PathBuf> {
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
fn parse_numstat(output: &str) -> Result<Vec<FileChange>> {
    output
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|record| {
            let mut parts = record.splitn(3, '\t');
            let added = parts.next().unwrap_or("");
            let deleted = parts.next().unwrap_or("");
            let path = parts
                .next()
                .ok_or(Error::new("Invalid Git diff statistics."))?;
            valid_path(path)?;
            let number = |s: &str| {
                if s == "-" {
                    Ok(None)
                } else {
                    s.parse::<u64>()
                        .map(Some)
                        .map_err(|_| Error::new("Invalid Git diff statistics."))
                }
            };
            Ok(FileChange {
                path: path.into(),
                additions: number(added)?,
                deletions: number(deleted)?,
                untracked: false,
            })
        })
        .collect()
}
fn untracked_lines(path: &Path, budget: &mut u64) -> Result<Option<u64>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES || metadata.len() > *budget {
        return Ok(None);
    }
    *budget = budget.saturating_sub(metadata.len());
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES || bytes.contains(&0) {
        return Ok(None);
    }
    Ok(Some(
        bytes.iter().filter(|&&b| b == b'\n').count() as u64
            + u64::from(!bytes.is_empty() && bytes.last() != Some(&b'\n')),
    ))
}
fn compare(cwd: &Path) -> Result<Comparison> {
    let root = PathBuf::from(
        git(cwd, &["rev-parse", "--show-toplevel"])
            .map_err(|_| Error::new("This agent's working directory is not a Git checkout."))?
            .trim_end_matches('\n'),
    );
    let base_ref = ["refs/remotes/origin/main", "refs/heads/main"]
        .into_iter()
        .find(|name| {
            git(
                &root,
                &["rev-parse", "--verify", &format!("{name}^{{commit}}")],
            )
            .is_ok()
        })
        .ok_or(Error::new(
            "No main branch is available locally. Fetch main, then retry.",
        ))?;
    let base = git(&root, &["merge-base", base_ref, "HEAD"])
        .map_err(|_| Error::new("This branch has no shared ancestor with main."))?
        .trim()
        .to_owned();
    let branch = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_owned();
    let mut files = parse_numstat(&git(
        &root,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--numstat",
            "-z",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            &base,
            "--",
        ],
    )?)?;
    let untracked = git(&root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut budget = 20 * 1024 * 1024;
    for name in untracked.split('\0').filter(|s| !s.is_empty()) {
        let path = valid_path(name)?;
        // A symlink is reported, but its target is never read as an untracked file.
        let additions = untracked_lines(&root.join(path), &mut budget)?;
        files.push(FileChange {
            path: name.into(),
            additions,
            deletions: additions.map(|_| 0),
            untracked: true,
        });
        if files.len() > MAX_FILES {
            break;
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Comparison {
        root,
        base,
        base_ref: base_ref
            .trim_start_matches("refs/")
            .trim_start_matches("remotes/")
            .trim_start_matches("heads/")
            .into(),
        branch,
        files,
    })
}
fn overview(cwd: &Path) -> Result<Value> {
    let comparison = compare(cwd)?;
    let added: u64 = comparison.files.iter().filter_map(|f| f.additions).sum();
    let deleted: u64 = comparison.files.iter().filter_map(|f| f.deletions).sum();
    Ok(
        json!({"root":comparison.root,"branch":comparison.branch,"baseRef":comparison.base_ref,"baseCommit":comparison.base,"additions":added,"deletions":deleted,"truncated":comparison.files.len()>MAX_FILES,"files":comparison.files.into_iter().take(MAX_FILES).collect::<Vec<_>>()}),
    )
}
fn session_directory(repo: &Repository, id: &str, requested: &str) -> Result<String> {
    let state = repo.state.lock().map_err(|_| Error::io())?;
    let record = arr(&state["sessions"])
        .into_iter()
        .find(|s| s["id"] == id)
        .ok_or(Error::new("Agent session not found."))?;
    if requested.is_empty() {
        return Ok(text(&record, "cwd").to_owned());
    }
    let canonical = Path::new(requested).canonicalize().ok();
    let same_checkout = |path: &str| !path.is_empty() && canonical.is_some() && Path::new(path).canonicalize().ok() == canonical;
    if same_checkout(text(&record, "cwd"))
        || arr(&record["worktrees"])
            .iter()
            .any(|t| same_checkout(text(t,"path")))
    {
        return Ok(requested.to_owned());
    }
    Err(Error::new(
        "This checkout is not linked to the agent session.",
    ))
}
pub fn for_session(repo: &Repository, id: &str, requested: &str) -> Result<Value> {
    let cwd = session_directory(repo, id, requested)?;
    let mut cache = repo.git_diff_cache.lock().unwrap();
    if let Some((path, at, value)) = &cache.entry {
        if path == &cwd && at.elapsed() < Duration::from_secs(8) {
            return Ok(value.clone());
        }
    }
    let value = overview(Path::new(&cwd))?;
    cache.entry = Some((cwd, Instant::now(), value.clone()));
    Ok(value)
}
pub fn review_version(repo: &Repository, id: &str, requested: &str) -> Result<Value> {
    use std::os::unix::fs::MetadataExt;
    let cwd=session_directory(repo,id,requested)?;
    let identity=git(Path::new(&cwd),&["rev-parse","--show-toplevel","HEAD"])?;
    let root=PathBuf::from(identity.lines().next().ok_or(Error::new("Git root unavailable."))?);
    let status=git(&root,&["status","--porcelain=v1","--untracked-files=all","-z"])?;
    let main=git(&root,&["rev-parse","--verify","refs/remotes/origin/main"]).or_else(|_|git(&root,&["rev-parse","--verify","refs/heads/main"]))?;
    let mut stamp=identity+&main+&status;
    let mut entries=status.split('\0').filter(|s|!s.is_empty());let mut count=0;
    while let Some(entry)=entries.next() {
        count+=1;if count>MAX_FILES{return Err(Error::new("Too many files to verify review freshness."));}
        let path=entry.get(3..).ok_or(Error::new("Invalid Git status."))?;
        match fs::symlink_metadata(root.join(valid_path(path)?)) {
            Ok(meta)=>stamp.push_str(&format!("{}:{}:{}:{}:{}:{}",meta.ino(),meta.len(),meta.mtime(),meta.mtime_nsec(),meta.ctime(),meta.ctime_nsec())),
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>stamp.push_str("deleted"),
            Err(error)=>return Err(error.into()),
        }
        if entry[..2].contains(['R','C']) { entries.next(); }
    }
    Ok(json!({"root":root,"version":context_kit::sha256(stamp.as_bytes())}))
}
pub fn review_file_stamp(root: &str, path: &str) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let target=Path::new(root).join(valid_path(path).ok()?);
    match fs::symlink_metadata(target) {
        Ok(m)=>Some(format!("{}:{}:{}:{}:{}:{}",m.ino(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec())),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Some("deleted".into()),
        Err(_)=>None,
    }
}
pub fn file_for_session(repo: &Repository, id: &str, path: &str, requested: &str) -> Result<Value> {
    let cwd = session_directory(repo, id, requested)?;
    let comparison = compare(Path::new(&cwd))?;
    let file = comparison
        .files
        .iter()
        .find(|f| f.path == path)
        .ok_or(Error::new(
            "This file is no longer in the diff. Refresh the file list.",
        ))?;
    let relative = valid_path(path)?;
    let target = comparison.root.join(&relative);
    let spec = format!("{}:{}", comparison.base, path);
    let limit = 512 * 1024;
    let old = if file.untracked || git(&comparison.root, &["cat-file", "-e", &spec]).is_err() {
        vec![]
    } else {
        let size = git(&comparison.root, &["cat-file", "-s", &spec])?
            .trim()
            .parse::<usize>()
            .map_err(|_| Error::new("Invalid Git object size."))?;
        if size > limit {
            return Err(Error::new(
                "This file exceeds the 512 KiB inline diff limit. Use Zed.",
            ));
        }
        terminal::run_bytes(
            "git",
            &[
                "-C".into(),
                comparison.root.to_string_lossy().into_owned(),
                "cat-file".into(),
                "blob".into(),
                spec,
            ],
            None,
            10,
        )?
    };
    let new = match fs::symlink_metadata(&target) {
        Ok(meta) if meta.is_file() && meta.len() <= limit as u64 => {
            working_file_bytes(&comparison.root,path,&meta,limit as u64)?
        }
        Ok(_) => return Err(Error::new("This file cannot be shown inline. Use Zed.")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(error) => return Err(error.into()),
    };
    if new.len() > limit || old.contains(&0) || new.contains(&0) {
        return Err(Error::new(
            "Binary or large files cannot be shown inline. Use Zed.",
        ));
    }
    let decode = |bytes: Vec<u8>| {
        String::from_utf8(bytes).map_err(|_| Error::new("This file is not UTF-8 text. Use Zed."))
    };
    let old = decode(old)?;
    let new = decode(new)?;
    if old.lines().count() > 10000 || new.lines().count() > 10000 {
        return Err(Error::new(
            "This file exceeds the 10,000-line inline diff limit. Use Zed.",
        ));
    }
    let fingerprint =
        context_kit::sha256(format!("{}\0{}\0{}\0{}", comparison.base, path, old, new).as_bytes());
    Ok(
        json!({"path":path,"root":comparison.root,"baseCommit":comparison.base,"baseRef":comparison.base_ref,"branch":comparison.branch,"fingerprint":fingerprint,"oldContent":old,"newContent":new}),
    )
}
fn command_args<'a>(args:&'a Value,file:bool)->Result<(&'a str,&'a str,&'a str)>{let object=args.as_object().ok_or(Error::new("Chat diff arguments must be an object."))?;let allowed=if file{&["recordId","cwd","path"][..]}else{&["recordId","cwd"][..]};if object.keys().any(|key|!allowed.contains(&key.as_str())){return Err(Error::new("Chat diffs accept only a saved chat ID, linked checkout and optional exact diff path."));}let id=args["recordId"].as_str().filter(|value|!value.is_empty()&&value.len()<=80&&!value.chars().any(char::is_control)).ok_or(Error::new("Supply the exact saved chat ID."))?;let cwd=match args.get("cwd"){None=>"",Some(value)=>value.as_str().filter(|value|value.len()<=4096&&!value.contains('\0')).ok_or(Error::new("Supply a linked checkout path."))?};let path=if file{args["path"].as_str().filter(|value|!value.is_empty()&&value.len()<=4096&&!value.contains('\0')).ok_or(Error::new("Supply an exact listed diff path."))?}else{""};Ok((id,cwd,path))}
fn command_identity(repo:&Repository,value:&Value,max:usize)->Result<Value>{let value=value.as_str().filter(|value|value.len()<=max&&!value.contains('\0')).ok_or(Error::new("Diff metadata exceeds the command limit."))?;let mut safe=value.to_owned();super::commands::redact_text(repo,&mut safe);if safe!=value{return Err(Error::new("Diff identity cannot be safely projected."));}Ok(json!(value))}
pub fn command_overview(repo:&Repository,args:&Value)->Result<Value>{let(id,cwd,_)=command_args(args,false)?;command_identity(repo,&json!(id),80)?;let value=for_session(repo,id,cwd)?;let mut result=json!({"recordId":id});for field in ["root","branch","baseRef","baseCommit"]{result[field]=command_identity(repo,&value[field],4096)?;}for field in ["additions","deletions"]{result[field]=value[field].as_u64().map_or(Value::Null,|value|json!(value));}result["truncated"]=json!(value["truncated"]==true);let files=arr(&value["files"]);if files.len()>MAX_FILES{return Err(Error::new("Diff overview exceeds 500 files."));}result["files"]=json!(files.into_iter().map(|row|Ok(json!({"path":command_identity(repo,&row["path"],4096)?,"additions":row["additions"].as_u64(),"deletions":row["deletions"].as_u64(),"untracked":row["untracked"]==true}))).collect::<Result<Vec<Value>>>()?);Ok(result)}
pub fn command_file(repo:&Repository,args:&Value)->Result<Value>{let(id,cwd,path)=command_args(args,true)?;command_identity(repo,&json!(id),80)?;command_identity(repo,&json!(path),4096)?;let value=file_for_session(repo,id,path,cwd)?;let mut result=json!({"recordId":id});for field in ["root","path","branch","baseRef","baseCommit","fingerprint"]{result[field]=command_identity(repo,&value[field],4096)?;}let mut redacted=false;for field in ["oldContent","newContent"]{let raw=value[field].as_str().ok_or(Error::new("Diff text is unavailable."))?;let mut content=raw.to_owned();super::commands::redact_text(repo,&mut content);redacted|=content!=raw;if content.len()>512*1024{return Err(Error::new("Redacted diff text exceeds 512 KiB."));}result[field]=json!(content);}result["contentRedacted"]=json!(redacted);Ok(result)}
fn working_file_bytes(root:&Path,path:&str,metadata:&fs::Metadata,limit:u64)->Result<Vec<u8>>{super::local_files::read(root,path,limit,Some(&super::local_files::generation(metadata)))?.ok_or(Error::new("The diff file changed. Refresh its file list."))}
fn snapshot_file(root: &Path, relative: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let directory = root
        .join(context_kit::sha256(bytes))
        .join(context_kit::sha256(relative.as_os_str().as_encoded_bytes()));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    let path = directory.join(
        relative
            .file_name()
            .ok_or(Error::new("Invalid diff file name."))?,
    );
    use std::io::Write;
    match fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
    {
        Ok(mut file) => file.write_all(bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !fs::symlink_metadata(&path)?.is_file() {
                return Err(Error::new("Invalid cached diff file."));
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(path)
}
pub fn zed_arguments(cwd: &Path, snapshots: &Path) -> Result<Vec<String>> {
    let comparison = compare(cwd)?;
    if comparison.files.len() > MAX_FILES {
        return Err(Error::new(
            "More than 500 changed files. Open a smaller diff in Zed.",
        ));
    }
    if comparison.files.is_empty() {
        return Err(Error::new("No changes against main in this checkout."));
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(snapshots)?;
    let result = (|| {
        let mut args = vec![];
        let mut total = 0;
        for file in &comparison.files {
            let relative = valid_path(&file.path)?;
            let target = comparison.root.join(&relative);
            let spec = format!("{}:{}", comparison.base, file.path);
            let bytes = if file.untracked {
                vec![]
            } else if git(&comparison.root, &["cat-file", "-e", &spec]).is_err() {
                vec![]
            } else {
                let size = git(&comparison.root, &["cat-file", "-s", &spec])?
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| Error::new("Invalid Git object size."))?;
                if size > MAX_FILE_BYTES {
                    return Err(Error::new(format!(
                        "{} exceeds the 10 MiB diff limit.",
                        file.path
                    )));
                }
                terminal::run_bytes(
                    "git",
                    &[
                        "-C".into(),
                        comparison.root.to_string_lossy().into_owned(),
                        "cat-file".into(),
                        "blob".into(),
                        spec,
                    ],
                    None,
                    10,
                )?
            };
            total += bytes.len();
            if total > 100 * 1024 * 1024 {
                return Err(Error::new("The diff exceeds the 100 MiB snapshot limit."));
            }
            let old = snapshot_file(snapshots, &relative, &bytes)?;
            let new = match fs::symlink_metadata(&target) {
                Ok(meta) if meta.file_type().is_symlink() => snapshot_file(
                    snapshots,
                    &relative,
                    fs::read_link(&target)?.to_string_lossy().as_bytes(),
                )?,
                Ok(meta) if meta.is_file() => {
                    if meta.len() > MAX_FILE_BYTES {
                        return Err(Error::new(format!(
                            "{} exceeds the 10 MiB diff limit.",
                            file.path
                        )));
                    }
                    let real = target.canonicalize()?;
                    if !real.starts_with(&comparison.root) {
                        return Err(Error::new("A changed file points outside the checkout."));
                    }
                    target
                }
                Ok(_) => {
                    return Err(Error::new(
                        "A changed submodule cannot be opened as a file diff.",
                    ))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    snapshot_file(snapshots, &relative, &[])?
                }
                Err(error) => return Err(error.into()),
            };
            args.extend([
                "--diff".into(),
                old.to_string_lossy().into_owned(),
                new.to_string_lossy().into_owned(),
            ]);
        }
        args.push(comparison.root.to_string_lossy().into_owned());
        Ok(args)
    })();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn commit(root: &Path) {
        git(root, &["add", "-A"]).unwrap();
        git(
            root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "fixture",
            ],
        )
        .unwrap();
    }
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo with spaces");
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        git(&root, &["init", "-b", "main"]).unwrap();
        fs::write(root.join("a.txt"), b"base\nkeep\n").unwrap();
        fs::write(root.join("deleted.txt"), b"deleted\n").unwrap();
        fs::write(root.join("binary.bin"), [0, 1, 2]).unwrap();
        commit(&root);
        git(&root, &["checkout", "-b", "feature"]).unwrap();
        fs::write(root.join("a.txt"), b"branch\nkeep\n").unwrap();
        fs::write(root.join("committed.txt"), b"committed\n").unwrap();
        commit(&root);
        git(&root, &["checkout", "main"]).unwrap();
        fs::write(root.join("main-only.txt"), b"not part of this work\n").unwrap();
        commit(&root);
        git(&root, &["checkout", "feature"]).unwrap();
        fs::write(root.join("a.txt"), b"branch\nkeep\nlocal\n").unwrap();
        fs::remove_file(root.join("deleted.txt")).unwrap();
        fs::write(root.join("binary.bin"), [0, 2, 3]).unwrap();
        fs::write(root.join("new\tfile.csv"), b"col\n1\n").unwrap();
        (temp, root)
    }
    #[test]
    fn overview_includes_committed_and_dirty_files_but_excludes_main_only_changes() {
        let (_temp, root) = fixture();
        let data = overview(&root).unwrap();
        let files = arr(&data["files"]);
        assert_eq!(data["branch"], "feature");
        assert_eq!(data["baseRef"], "main");
        assert_eq!(files.len(), 5);
        let a = files.iter().find(|f| f["path"] == "a.txt").unwrap();
        assert_eq!(a["additions"], 2);
        assert_eq!(a["deletions"], 1);
        let binary = files.iter().find(|f| f["path"] == "binary.bin").unwrap();
        assert!(binary["additions"].is_null());
        let untracked = files.iter().find(|f| f["path"] == "new\tfile.csv").unwrap();
        assert_eq!(untracked["additions"], 2);
        assert_eq!(untracked["untracked"], true);
        assert!(!files.iter().any(|f| f["path"] == "main-only.txt"));
    }
    #[test]
    fn zed_diff_uses_base_snapshots_and_live_worktree_files_without_changing_git() {
        let (temp, root) = fixture();
        let before = git(&root, &["status", "--porcelain=v1", "-z"]).unwrap();
        let args = zed_arguments(&root, &temp.path().join("snapshots")).unwrap();
        assert_eq!(args.last().unwrap(), root.to_str().unwrap());
        assert_eq!(args.len(), 16);
        let pairs = args[..args.len() - 1].chunks_exact(3).collect::<Vec<_>>();
        let a = pairs
            .iter()
            .find(|p| p[2] == root.join("a.txt").to_str().unwrap())
            .unwrap();
        assert_eq!(a[0], "--diff");
        assert_eq!(fs::read(&a[1]).unwrap(), b"base\nkeep\n");
        let deleted = pairs
            .iter()
            .find(|p| p[1].ends_with("/deleted.txt"))
            .unwrap();
        assert_eq!(fs::read(&deleted[2]).unwrap(), b"");
        let binary = pairs
            .iter()
            .find(|p| p[1].ends_with("/binary.bin"))
            .unwrap();
        assert_eq!(fs::read(&binary[1]).unwrap(), [0, 1, 2]);
        assert_eq!(
            git(&root, &["status", "--porcelain=v1", "-z"]).unwrap(),
            before
        );
    }
    #[test]
    fn overview_uses_the_selected_worktree_and_leaves_other_checkouts_out() {
        let (temp, root) = fixture();
        let tree = temp.path().join("agent-worktree");
        git(
            &root,
            &[
                "worktree",
                "add",
                "-b",
                "agent-worktree",
                tree.to_str().unwrap(),
                "HEAD",
            ],
        )
        .unwrap();
        fs::write(tree.join("only-in-worktree.rs"), b"worktree\n").unwrap();
        let data = overview(&tree).unwrap();
        let files = arr(&data["files"]);
        assert_eq!(data["branch"], "agent-worktree");
        assert!(files.iter().any(|f| f["path"] == "only-in-worktree.rs"));
        assert!(!files.iter().any(|f| f["path"] == "new\tfile.csv"));
        let args = zed_arguments(&tree, &temp.path().join("snapshots")).unwrap();
        assert_eq!(
            PathBuf::from(args.last().unwrap()),
            tree.canonicalize().unwrap()
        );
    }
    #[test]
    fn inline_file_preview_is_scoped_to_the_session_diff_and_fingerprints_content() {
        let (temp, root) = fixture();
        let home = temp.path().join("mydash");
        fs::create_dir(&home).unwrap();
        let repo = Repository::open(home, None, true).unwrap();
        repo.transaction(|s| {
            s["sessions"] = json!([{"id":"review-agent","cwd":root}]);
            Ok(())
        })
        .unwrap();
        let first = file_for_session(&repo, "review-agent", "a.txt", "").unwrap();
        assert_eq!(first["oldContent"], "base\nkeep\n");
        assert_eq!(first["newContent"], "branch\nkeep\nlocal\n");
        fs::write(root.join("a.txt"), b"changed again\n").unwrap();
        let next = file_for_session(&repo, "review-agent", "a.txt", "").unwrap();
        assert_ne!(first["fingerprint"], next["fingerprint"]);
        assert!(file_for_session(&repo, "review-agent", "../../escape", "").is_err());
        assert!(file_for_session(&repo, "other-agent", "a.txt", "").is_err());
        assert!(file_for_session(&repo, "review-agent", "binary.bin", "").is_err());
        assert_eq!(
            file_for_session(&repo, "review-agent", "new\tfile.csv", "").unwrap()["oldContent"],
            ""
        );
    }
    #[test]
    fn session_diff_can_select_only_its_linked_checkouts() {
        let (temp, root) = fixture();
        let tree = temp.path().join("second");
        git(
            &root,
            &[
                "worktree",
                "add",
                "-b",
                "second",
                tree.to_str().unwrap(),
                "HEAD",
            ],
        )
        .unwrap();
        std::fs::write(tree.join("only-second.rs"), "fn second() {}\n").unwrap();
        let repo = Repository::open(temp.path().join("mydash"), None, true).unwrap();
        repo.transaction(|s| {
            s["sessions"] = json!([{"id":"agent","cwd":root,"worktrees":[{"path":tree}]}]);
            Ok(())
        })
        .unwrap();
        let overview = for_session(&repo, "agent", tree.to_str().unwrap()).unwrap();
        assert_eq!(overview["branch"], "second");
        assert!(arr(&overview["files"])
            .iter()
            .any(|f| f["path"] == "only-second.rs"));
        let file =
            file_for_session(&repo, "agent", "only-second.rs", tree.to_str().unwrap()).unwrap();
        assert_eq!(file["newContent"], "fn second() {}\n");
        assert!(for_session(&repo, "agent", temp.path().to_str().unwrap()).is_err());
        assert!(file_for_session(&repo, "agent", "only-second.rs", "").is_err());
    }
    #[test]
    fn missing_main_and_invalid_paths_report_errors_instead_of_guessing() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init", "-b", "trunk"]).unwrap();
        fs::write(temp.path().join("a"), b"x").unwrap();
        commit(temp.path());
        assert!(overview(temp.path())
            .unwrap_err()
            .message
            .contains("No main branch"));
        assert!(parse_numstat("1\t0\t../../escape\0").is_err());
        assert!(parse_numstat("1\t0\t/absolute\0").is_err());
    }
    #[test]
    fn chat_diff_commands_reuse_linked_owner_and_whitelist_cached_overview(){let (temp,root)=fixture();let repo=Repository::open(temp.path().join("mydash"),None,true).unwrap();repo.transaction(|s|{s["sessions"]=json!([{"id":"command-agent","cwd":root,"privateNs":"private-diff-canary"}]);Ok(())}).unwrap();let bytes=fs::read(&repo.data).unwrap();let initial=for_session(&repo,"command-agent","").unwrap();let mut cached=initial.clone();cached["privateNs"]=json!("private-diff-canary");cached["files"][0]["privateNs"]=json!("private-diff-canary");repo.git_diff_cache.lock().unwrap().entry=Some((root.to_string_lossy().into_owned(),Instant::now(),cached));let result=super::super::commands::invoke(&repo,&json!({"name":"chats.diff","args":{"recordId":"command-agent"}}),false).unwrap();assert_eq!(result["root"],initial["root"]);assert_eq!(result["files"],initial["files"]);assert!(!result.to_string().contains("private-diff-canary"));let file=super::super::commands::invoke(&repo,&json!({"name":"chats.diff.file","args":{"recordId":"command-agent","path":"a.txt"}}),false).unwrap();assert_eq!(file["oldContent"],"base\nkeep\n");assert_eq!(file["newContent"],"branch\nkeep\nlocal\n");assert_eq!(file["contentRedacted"],false);assert_eq!(command_file(&repo,&json!({"recordId":"command-agent","path":"deleted.txt"})).unwrap()["newContent"],"");for args in [json!({"recordId":"command-agent","cwd":temp.path()}),json!({"recordId":"other-agent"}),json!({"recordId":"command-agent","root":root})]{assert!(command_overview(&repo,&args).is_err());}for path in ["../escape","/absolute","not-listed"]{assert!(command_file(&repo,&json!({"recordId":"command-agent","path":path})).is_err());}assert_eq!(fs::read(&repo.data).unwrap(),bytes);}
    #[test]
    fn chat_diff_anchored_reader_rejects_swapped_symlink_and_changed_generation(){let temp=tempfile::tempdir().unwrap();let root=temp.path().join("root");fs::create_dir(&root).unwrap();let file=root.join("code.txt");fs::write(&file,"captured source").unwrap();let captured=fs::symlink_metadata(&file).unwrap();let outside=temp.path().join("outside");fs::write(&outside,"private outside source").unwrap();fs::remove_file(&file).unwrap();std::os::unix::fs::symlink(&outside,&file).unwrap();assert!(working_file_bytes(&root,"code.txt",&captured,512*1024).is_err());fs::remove_file(&file).unwrap();fs::write(&file,"replacement source").unwrap();assert!(working_file_bytes(&root,"code.txt",&captured,512*1024).is_err());let current=fs::symlink_metadata(&file).unwrap();assert_eq!(working_file_bytes(&root,"code.txt",&current,512*1024).unwrap(),b"replacement source");}
    #[test]
    fn chat_diff_command_limits_binary_symlink_size_lines_and_overview_rows(){let (temp,root)=fixture();let repo=Repository::open(temp.path().join("mydash"),None,true).unwrap();repo.transaction(|s|{s["sessions"]=json!([{"id":"bounded-agent","cwd":root}]);Ok(())}).unwrap();fs::write(root.join("large.txt"),vec![b'x';512*1024+1]).unwrap();fs::write(root.join("lines.txt"),"x\n".repeat(10001)).unwrap();std::os::unix::fs::symlink(temp.path(),root.join("symlink-file")).unwrap();for path in ["large.txt","lines.txt","binary.bin","symlink-file"]{assert!(command_file(&repo,&json!({"recordId":"bounded-agent","path":path})).is_err());}for index in 0..501{fs::write(root.join(format!("bounded-{index}.txt")),b"").unwrap();}let result=command_overview(&repo,&json!({"recordId":"bounded-agent"})).unwrap();assert_eq!(arr(&result["files"]).len(),500);assert_eq!(result["truncated"],true);}
    #[test]
    fn chat_diff_command_redacts_source_but_preserves_original_fingerprint(){let (temp,root)=fixture();let home=temp.path().join("mydash");fs::create_dir(&home).unwrap();let secret="fixture-source-secret";fs::write(home.join(".env"),format!("MYDASH_TEST_SECRET={secret}\n")).unwrap();fs::write(root.join("a.txt"),format!("before\n{secret}\nafter\n")).unwrap();let repo=Repository::open(home,None,true).unwrap();repo.transaction(|s|{s["sessions"]=json!([{"id":"redacted-agent","cwd":root}]);Ok(())}).unwrap();let raw=file_for_session(&repo,"redacted-agent","a.txt","").unwrap();let result=super::super::commands::invoke(&repo,&json!({"name":"chats.diff.file","args":{"recordId":"redacted-agent","path":"a.txt"}}),false).unwrap();assert_eq!(result["fingerprint"],raw["fingerprint"]);assert_eq!(result["contentRedacted"],true);assert!(!result.to_string().contains(secret));assert!(text(&result,"newContent").contains("redacted"));}

    #[test]
    fn chat_diff_rejects_an_unlinked_valid_checkout_before_cache_and_accepts_explicit_link() {
        let(temp,root)=fixture();let(_other,other)=fixture();let repo=Repository::open(temp.path().join("profile"),None,true).unwrap();repo.transaction(|s|{s["sessions"]=json!([{"id":"linked-agent","cwd":root}]);Ok(())}).unwrap();
        let other_overview=overview(&other).unwrap();repo.git_diff_cache.lock().unwrap().entry=Some((other.to_string_lossy().into_owned(),Instant::now(),other_overview));let bytes=fs::read(&repo.data).unwrap();
        assert!(command_overview(&repo,&json!({"recordId":"linked-agent","cwd":other})).is_err());assert!(command_file(&repo,&json!({"recordId":"linked-agent","cwd":other,"path":"a.txt"})).is_err());assert_eq!(fs::read(&repo.data).unwrap(),bytes);
        repo.transaction(|s|{s["sessions"][0]["worktrees"]=json!([{"path":other}]);Ok(())}).unwrap();assert_eq!(command_overview(&repo,&json!({"recordId":"linked-agent","cwd":other})).unwrap()["root"],json!(other));
    }

}
