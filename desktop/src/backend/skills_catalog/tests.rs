use super::*;
use crate::backend::arr;
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
        vec![b'x'; usize::try_from(MAX_FILE_BYTES).unwrap() + 1],
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
