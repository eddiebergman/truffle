use super::*;

use super::work_search::normalized;
fn rank(row: &Value, words: &[String]) -> usize {
    let fields = [
        text(row, "title").to_owned(),
        text(row, "cwd").to_owned(),
        arr(&row["items"])
            .iter()
            .map(|i| {
                format!(
                    "{} {} {}",
                    text(i, "reference"),
                    text(i, "title"),
                    text(i, "url")
                )
            })
            .collect::<Vec<_>>()
            .join(" "),
    ];
    words
        .iter()
        .map(|word| {
            fields
                .iter()
                .enumerate()
                .filter_map(|(index, field)| {
                    let compact = normalized(field);
                    if compact.contains(word) {
                        Some(if index == 0 { 4 } else { 2 })
                    } else if word.len() >= 5
                        && field
                            .split(|c: char| !c.is_alphanumeric())
                            .any(|token| token.len() >= 5 && word.starts_with(&normalized(token)))
                    {
                        Some(1)
                    } else {
                        None
                    }
                })
                .max()
                .unwrap_or(0)
        })
        .sum()
}
fn rows(repo: &Repository) -> Vec<Value> {
    let candidates = arr(&repo.inventory()["candidates"]);
    let state = repo.state.lock().unwrap();
    let mut rows = arr(&state["sessions"]).iter().map(|r|json!({"id":r["id"],"title":r["title"],"agent":r["agent"],"cwd":r["cwd"],"items":r["items"],"archived":r["archivedAt"].as_f64().is_some_and(|t|t>0.0),"live":candidates.iter().any(|c|c["live"]==true&&(c["recordId"]==r["id"]||!text(r,"sessionId").is_empty()&&c["sessionId"]==r["sessionId"]&&c["agent"]==r["agent"]))})).collect::<Vec<_>>();
    for c in candidates.iter().filter(|c| c["live"] == true) {
        if rows.iter().any(|r| r["id"] == c["recordId"]) {
            continue;
        }
        if arr(&state["sessions"]).iter().any(|r| {
            !text(r, "sessionId").is_empty()
                && r["sessionId"] == c["sessionId"]
                && r["agent"] == c["agent"]
        }) {
            continue;
        }
        let id = if c["host"] == "embedded" {
            format!("embedded:{}", text(c, "recordId"))
        } else {
            format!("{}:{}", text(c, "agent"), text(c, "sessionId"))
        };
        rows.push(json!({"id":id,"title":c["title"],"cwd":c["cwd"],"agent":c["agent"],"items":[],"archived":false,"live":true}));
    }
    rows
}
pub fn search(repo: &Repository, args: &Value) -> Result<Value> {
    let query = checked(args, "query", 500, false)?;
    let words = query
        .split_whitespace()
        .map(normalized)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>();
    let mut found = rows(repo)
        .into_iter()
        .filter_map(|row| {
            let score = rank(&row, &words);
            (words.is_empty() || score > 0).then_some((score, row))
        })
        .collect::<Vec<_>>();
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| text(&a.1, "title").cmp(text(&b.1, "title")))
    });
    let total = found.len();
    let offset = args["offset"].as_u64().unwrap_or(0).min(100000) as usize;
    Ok(
        json!({"chats":found.into_iter().skip(offset).take(40).map(|(_,row)|row).collect::<Vec<_>>(),"total":total,"offset":offset,"includes":"live, snoozed, saved and archived chats","matching":"Ranked title, working folder and linked work; ignores punctuation. Results can match only some query words. Verify the title and folder before opening.","note":"No terminal transcript is searched. If no match appears, try the folder, project name, or a shorter keyword."}),
    )
}
pub fn target(repo: &Repository, id: &str) -> Result<Value> {
    rows(repo)
        .into_iter()
        .find(|row| row["id"] == id)
        .ok_or(Error::new("That chat is unavailable. Search chats again."))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_finds_punctuation_variants_in_folders_and_archived_chats() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        repo.transaction(|s|{s["sessions"]=json!([{"id":uuid(),"title":"benchmark","cwd":"/projects/server/.worktrees/thinking-v2-repro","items":[],"archivedAt":123},{"id":uuid(),"title":"Unrelated","cwd":"/projects/other","items":[]}]);Ok(())}).unwrap();
        let found = search(&repo, &json!({"query":"benchmarking thinkingv2"})).unwrap();
        assert_eq!(found["total"], 1);
        assert_eq!(found["chats"][0]["title"], "benchmark");
        assert_eq!(found["chats"][0]["archived"], true);
        assert!(target(&repo, "missing").is_err());
        assert_eq!(
            search(&repo, &json!({"query":"nothingmatching"})).unwrap()["total"],
            0
        );
    }
}
