use super::*;
use std::{
    collections::{BTreeMap, HashSet},
    io::Read,
    path::Path,
};

const MAX_SKILLS: usize = 1024;
const MAX_ENTRIES: usize = 8192;
const MAX_FILE_BYTES: u64 = 65536;

struct Root {
    path: PathBuf,
    agent: &'static str,
    scope: &'static str,
}

pub fn review_skill(configuration: &Path) -> Value {
    let path = configuration.join("skills/truffle-review/SKILL.md");
    json!({"name":"truffle-review","path":path,"available":path.is_file(),"commands":{"claude":"/truffle-review","codex":"$truffle-review"}})
}

/// Read available skill metadata without importing or running the skill.
pub fn discover(user_home: &Path, projects: &[PathBuf], application_skills: &Path) -> Value {
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home.join(".claude"));
    let codex = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home.join(".codex"));
    let mut roots = vec![
        Root {
            path: claude.join("skills"),
            agent: "claude",
            scope: "user",
        },
        Root {
            path: codex.join("skills"),
            agent: "codex",
            scope: "user",
        },
        Root {
            path: user_home.join(".agents/skills"),
            agent: "codex",
            scope: "user",
        },
    ];
    for project in projects
        .iter()
        .take(20)
        .flat_map(|p| project_directories(p))
    {
        for (dir, agent) in [
            (".claude/skills", "claude"),
            (".agents/skills", "codex"),
            (".codex/skills", "codex"),
        ] {
            roots.push(Root {
                path: project.join(dir),
                agent,
                scope: "project",
            });
        }
    }
    for agent in ["claude", "codex"] {
        roots.push(Root {
            path: application_skills.to_path_buf(),
            agent,
            scope: "application",
        });
    }
    let mut seen = HashSet::new();
    roots.retain(|root| seen.insert((root.path.clone(), root.agent)));
    scan(&roots)
}

fn project_directories(project: &Path) -> Vec<PathBuf> {
    let ancestors = project.ancestors().take(16).collect::<Vec<_>>();
    match ancestors.iter().position(|path| path.join(".git").exists()) {
        Some(index) => ancestors[..=index]
            .iter()
            .map(|path| path.to_path_buf())
            .collect(),
        None => vec![project.to_path_buf()],
    }
}

fn scan(roots: &[Root]) -> Value {
    let mut entries = BTreeMap::<PathBuf, Value>::new();
    let mut remaining = MAX_ENTRIES;
    let mut truncated = false;
    for root in roots {
        let mut pending = vec![(root.path.clone(), 0)];
        let mut seen = HashSet::new();
        while let Some((dir, depth)) = pending.pop() {
            if remaining == 0 || entries.len() >= MAX_SKILLS {
                truncated = true;
                break;
            }
            let Ok(canonical) = dir.canonicalize() else {
                continue;
            };
            if !seen.insert(canonical) {
                continue;
            }
            let skill = dir.join("SKILL.md");
            if skill.is_file() {
                if let Ok(path) = skill.canonicalize() {
                    if let Some(existing) = entries.get_mut(&path) {
                        for (field, value) in [("agents", root.agent), ("sources", root.scope)] {
                            if !existing[field]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|v| v == value)
                            {
                                existing[field].as_array_mut().unwrap().push(json!(value));
                            }
                        }
                    } else if let Some((name, description)) = metadata(&skill) {
                        let command_name = if name
                            .chars()
                            .all(|c| c.is_alphanumeric() || "-_:.".contains(c))
                        {
                            name.clone()
                        } else {
                            dir.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string()
                        };
                        entries.insert(path.clone(),json!({"commandName":command_name,"name":name,"description":description,"path":path,"agents":[root.agent],"sources":[root.scope]}));
                    }
                }
                continue;
            }
            if depth >= 4 {
                continue;
            }
            let Ok(children) = std::fs::read_dir(&dir) else {
                continue;
            };
            for child in children {
                if remaining == 0 {
                    truncated = true;
                    break;
                }
                remaining -= 1;
                let Ok(child) = child else { continue };
                let path = child.path();
                if matches!(
                    child.file_name().to_str(),
                    Some(".trash" | ".git" | "node_modules")
                ) {
                    continue;
                }
                if path.is_dir() {
                    pending.push((path, depth + 1));
                }
            }
        }
    }
    let mut skills = entries.into_values().collect::<Vec<_>>();
    skills.sort_by(|a, b| {
        text(a, "name")
            .to_lowercase()
            .cmp(&text(b, "name").to_lowercase())
            .then_with(|| text(a, "path").cmp(text(b, "path")))
    });
    json!({"skills":skills,"truncated":truncated})
}

fn scalar(raw: &str) -> String {
    let value = raw.trim();
    if value.starts_with('"') {
        return serde_json::from_str::<String>(value).unwrap_or_else(|_| value.to_owned());
    }
    if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        return value[1..value.len() - 1].replace("''", "'");
    }
    value.to_owned()
}

fn metadata(path: &Path) -> Option<(String, String)> {
    let file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size > MAX_FILE_BYTES {
        return None;
    }
    let mut raw = String::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut raw)
        .ok()?;
    if raw.len() as u64 > MAX_FILE_BYTES {
        return None;
    }
    let fallback = path.parent()?.file_name()?.to_string_lossy().to_string();
    let mut name = fallback;
    let mut description = String::new();
    let mut lines = raw.lines();
    if lines.next() != Some("---") {
        return Some((name, description));
    }
    let mut block = false;
    for line in lines {
        if line == "---" {
            break;
        }
        if block && (line.starts_with(' ') || line.starts_with('\t')) {
            if !description.is_empty() {
                description.push(' ');
            }
            description.push_str(line.trim());
            continue;
        }
        block = false;
        if let Some(value) = line.strip_prefix("name:") {
            let parsed = scalar(value);
            if !parsed.is_empty() {
                name = parsed;
            }
        }
        if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim();
            block = matches!(value, "|" | ">" | "|-" | ">-" | "|+" | ">+");
            description = if block { String::new() } else { scalar(value) };
        }
    }
    Some((
        name.chars().take(180).collect(),
        description.chars().take(1000).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_review_is_discovered_for_both_agents_without_global_installation() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config");
        super::super::config::install(&config).unwrap();
        let result = discover(&temp.path().join("empty-home"), &[], &config.join("skills"));
        let path = config
            .join("skills/truffle-review/SKILL.md")
            .canonicalize()
            .unwrap();
        let skill = arr(&result["skills"])
            .into_iter()
            .find(|skill| skill["path"] == json!(path))
            .unwrap();
        assert_eq!(skill["commandName"], "truffle-review");
        assert_eq!(skill["agents"], json!(["claude", "codex"]));
        assert_eq!(skill["sources"], json!(["application"]));
        assert!(!temp.path().join("empty-home/.claude").exists());
        assert!(!temp.path().join("empty-home/.agents").exists());
    }

    #[test]
    fn reads_metadata_deduplicates_links_and_stops_cycles() {
        let temp = tempfile::tempdir().unwrap();
        let shared = temp.path().join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("SKILL.md"),"---\nname: 'Latency review'\ndescription: >-\n  Find slow paths.\n  Preserve context.\n---\nNever execute this.").unwrap();
        let claude = temp.path().join("claude");
        let codex = temp.path().join("codex");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(&codex).unwrap();
        std::os::unix::fs::symlink(&shared, claude.join("latency")).unwrap();
        std::os::unix::fs::symlink(&shared, codex.join("latency")).unwrap();
        std::os::unix::fs::symlink(&claude, claude.join("loop")).unwrap();
        let result = scan(&[
            Root {
                path: claude,
                agent: "claude",
                scope: "user",
            },
            Root {
                path: codex,
                agent: "codex",
                scope: "project",
            },
        ]);
        assert_eq!(result["skills"].as_array().unwrap().len(), 1);
        let skill = &result["skills"][0];
        assert_eq!(skill["name"], "Latency review");
        assert_eq!(skill["description"], "Find slow paths. Preserve context.");
        assert_eq!(skill["agents"], json!(["claude", "codex"]));
        assert_eq!(skill["sources"], json!(["user", "project"]));
        assert_eq!(result["truncated"], false);
    }
    #[test]
    fn project_roots_stop_at_git_boundary_and_trash_is_not_offered() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("repo");
        let nested = project.join("apps/api");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(project.join(".git"), "gitdir: ../main/.git/worktrees/repo").unwrap();
        assert_eq!(
            project_directories(&nested),
            vec![nested.clone(), project.join("apps"), project.clone()]
        );
        let root = temp.path().join("skills");
        std::fs::create_dir_all(root.join(".trash/old")).unwrap();
        std::fs::write(root.join(".trash/old/SKILL.md"), "Old skill").unwrap();
        let result = scan(&[Root {
            path: root,
            agent: "claude",
            scope: "user",
        }]);
        assert_eq!(result["skills"], json!([]));
    }
    #[test]
    fn skips_oversized_files_and_uses_folder_name_without_frontmatter() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for name in ["plain", "large"] {
            std::fs::create_dir(root.join(name)).unwrap();
        }
        std::fs::write(root.join("plain/SKILL.md"), "# Plain skill").unwrap();
        std::fs::write(
            root.join("large/SKILL.md"),
            vec![b'x'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let result = scan(&[Root {
            path: root.to_owned(),
            agent: "claude",
            scope: "user",
        }]);
        assert_eq!(result["skills"].as_array().unwrap().len(), 1);
        assert_eq!(result["skills"][0]["name"], "plain");
    }
}
