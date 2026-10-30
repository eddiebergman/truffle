use super::chat_preparation::{self, Created, Recipe};
use super::{
    arr, checked, commands, directory, expand, home, json, now, safe_url, sid, text, uuid, Error,
    PathBuf, Repository, Result, Value,
};
use super::{artifacts, domain, sessions, terminal};
use std::sync::Arc;
pub fn work_root() -> PathBuf {
    std::env::var("TRUFFLE_WORKDIR").map_or_else(|_| home().join("code"), |s| expand(&s))
}
pub fn folders() -> Value {
    folders_in(&work_root())
}
fn folders_in(root: &std::path::Path) -> Value {
    let mut result = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| {
            e.path()
                .canonicalize()
                .ok()
                .map(|p| json!({"name":e.file_name().to_string_lossy(),"path":p}))
        })
        .collect::<Vec<_>>();
    result.sort_by_key(|x| text(x, "name").to_lowercase());
    json!(result)
}
fn repo_name(item: &Value) -> String {
    reqwest::Url::parse(text(item, "url"))
        .ok()
        .map(|u| {
            u.path()
                .trim_matches('/')
                .split('/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default()
}
pub fn contexts(item: &Value) -> Vec<Value> {
    let mut result = vec![];
    match text(item,"source"){"github"=>{let name=repo_name(item);result.push(json!({"key":format!("github:{}",name.to_lowercase()),"label":format!("GitHub repo {name}")}));},"linear"=>{let mut parent=&item["parent"];let mut top=Value::Null;for _ in 0..8 {if parent.is_null()||parent.as_object().is_none(){break;}top=parent.clone();parent=&parent["parent"];}if top.is_null()&&!arr(&item["children"]).is_empty(){top=item.clone();}
if !top.is_null(){result.push(json!({"key":format!("linear-family:{}",text(&top,"id")),"label":format!("Issue family {}",text(&top,"reference"))}));}
if !text(item,"projectId").is_empty(){let workspace=text(item,"url").split('/').nth(3).unwrap_or("");result.push(json!({"key":format!("linear-project:{workspace}:{}",text(item,"projectId")),"label":format!("Linear project {}",text(item,"project"))}));}},"sentry"=>result.push(json!({"key":format!("sentry-project:{}",text(item,"project")),"label":format!("Sentry project {}",text(item,"project"))})),_=>()}
    if !text(item, "id").is_empty() {
        result
            .push(json!({"key":format!("item:{}",text(item,"id")),"label":"Only this work item"}));
    }
    result
}
/// Authoritative automatic choice, shared by launch and the read-only project resolver.
/// A saved path is an existing-directory hint; it never creates a folder or changes mappings.
pub(super) fn automatic_folder(
    s: &Value,
    item: &Value,
    root: &std::path::Path,
    candidates: &Value,
) -> Option<String> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let usable = |value: &str| directory(value).ok().filter(|path| *path != root);
    let setting = &s["itemSettings"][text(item, "id")];
    if setting["explicit"] == true {
        if let Some(path) = usable(text(setting, "cwd")) {
            return Some(path);
        }
    }
    for choice in contexts(item) {
        if let Some(path) = usable(text(&s["folderMappings"][text(&choice, "key")], "path")) {
            return Some(path);
        }
    }
    if item["source"] == "notes" {
        if let Some(path) = usable(text(item, "cwd")) {
            return Some(path);
        }
    }
    if item["source"] == "github" {
        let name = repo_name(item)
            .split('/')
            .next_back()
            .unwrap_or("")
            .to_lowercase();
        if let Some(path) = arr(candidates)
            .iter()
            .filter(|folder| text(folder, "name").to_lowercase() == name)
            .find_map(|folder| usable(text(folder, "path")))
        {
            return Some(path);
        }
    }
    usable(text(setting, "cwd"))
}
fn choose_with(
    s: &Value,
    item: &Value,
    p: &Value,
    root: &std::path::Path,
    candidates: &Value,
) -> Result<(String, Option<String>)> {
    let choices = contexts(item);
    let manual = text(p, "folderPath");
    let new = checked(p, "newFolder", 80, false)?;
    if !new.is_empty() {
        if !new
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            || !new.chars().next().unwrap().is_ascii_alphanumeric()
        {
            return Err(Error::new("Use a simple folder name without paths."));
        }
        let path = root.join(&new);
        if path.exists() {
            return Err(Error::new("That folder already exists."));
        }
        return Ok((path.to_string_lossy().into(), Some(new)));
    }
    if !manual.is_empty() {
        if !arr(candidates).iter().any(|x| x["path"] == manual) {
            return Err(Error::new(
                "Choose an existing folder under the work directory.",
            ));
        }
        return Ok((directory(manual)?, None));
    }
    if p["chooseFolder"] != true {
        if let Some(path) = automatic_folder(s, item, root, candidates) {
            return Ok((path, None));
        }
        if item.is_null() && root.is_dir() {
            return Ok((root.to_string_lossy().into(), None));
        }
    }
    let mut error = Error::new("Choose a folder for this session.");
    error.status = 409;
    error.data = json!({"location":{"root":root,"folders":candidates,"contexts":choices,"suggestedName":repo_name(item).split('/').next_back().unwrap_or(""),"title":item["title"].as_str().unwrap_or("New session"),"reason":"Choose where the agent should work. This can be remembered for related work."}});
    Err(error)
}
fn agent_schema() -> Value {
    json!({"type":"string","enum":["claude","codex"]})
}
fn checked_agent(p: &Value) -> Result<&str> {
    let agent = text(p, "agent");
    if !["claude", "codex"].contains(&agent) {
        return Err(Error::new("Choose Claude or Codex."));
    }
    Ok(agent)
}
fn resolve_urls(
    repo: &Repository,
    p: &Value,
    resolve: impl Fn(&str) -> Result<Value>,
) -> Result<Vec<Value>> {
    let mut imported = vec![];
    let urls = arr(&p["urls"]);
    if urls.len() > 10 {
        return Err(Error::new("Import at most 10 links."));
    }
    for url in urls {
        let cached = {
            let s = repo.state.lock().unwrap();
            domain::catalog(&s).into_values().find(|x| x["url"] == url)
        };
        imported.push(match cached {
            Some(v) => v,
            None => resolve(
                url.as_str()
                    .ok_or_else(|| Error::new("Invalid source URL."))?,
            )?,
        });
    }
    Ok(imported)
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveIntent {
    Save,
    Register,
}
/// Save `p` with its imported items and named project linked.
fn save_linked(s: &mut Value, p: &Value, imported: &[Value], intent: SaveIntent) -> Result<Value> {
    let mut payload = p.clone();
    let mut ids = arr(&p["itemIds"]);
    for item in imported {
        if intent == SaveIntent::Register && domain::catalog(s).contains_key(text(item, "id")) {
            ids.push(item["id"].clone());
            continue;
        }
        let mut items = arr(&s["items"]);
        items.retain(|x| x["id"] != item["id"]);
        items.push(item.clone());
        s["items"] = json!(items);
        ids.push(item["id"].clone());
    }
    let name = p["projectName"]
        .as_str()
        .or_else(|| p["project"].as_str())
        .unwrap_or("");
    if !name.trim().is_empty() {
        let project = arr(&s["notes"])
            .into_iter()
            .find(|x| x["title"] == name && x["archivedAt"].is_null());
        let id = if let Some(project) = project {
            text(&project, "id").to_owned()
        } else {
            domain::edit(
                s,
                &json!({"action":"save-note","title":name,"status":"todo","cwd":p["cwd"]}),
            )?;
            text(arr(&s["notes"]).last().unwrap(), "id").to_owned()
        };
        ids.push(json!(id));
    }
    payload["itemIds"] = json!(ids);
    match intent {
        SaveIntent::Save => {
            if text(&payload, "title").is_empty() {
                payload["title"] = json!(if name.is_empty() {
                    "Agent session"
                } else {
                    name
                });
            }
            sessions::save(s, &payload)
        }
        SaveIntent::Register => sessions::register(s, &payload),
    }
}
/// A new chat from a form, or with `attach`, a chat for an existing conversation.
pub(super) struct NewChat<'a> {
    intent: &'a Value,
    imported: Vec<Value>,
    attach: bool,
}
impl<'a> NewChat<'a> {
    pub(super) fn schema() -> Value {
        chat_preparation::schema(
            json!({"agent":agent_schema(),"title":{"type":"string","maxLength":180},"cwd":{"type":"string","maxLength":4096},"instruction":{"type":"string","maxLength":8000},"destination":{"type":"string","enum":["embedded","desktop","cmux"]},"itemIds":{"type":"array","maxItems":100,"items":{"type":"string","maxLength":200}},"urls":{"type":"array","maxItems":10,"items":{"type":"string","maxLength":4096}},"projectName":{"type":"string","maxLength":180}}),
            &[
                "agent",
                "title",
                "cwd",
                "destination",
                "instruction",
                "itemIds",
            ],
        )
    }
    pub(super) fn attach_schema() -> Value {
        chat_preparation::schema(
            json!({"agent":agent_schema(),"sessionId":chat_preparation::uuid_schema(),"title":{"type":"string","maxLength":180},"cwd":{"type":"string","maxLength":4096},"destination":{"type":"string","enum":["embedded","desktop","cmux"]},"itemIds":{"type":"array","maxItems":100,"items":{"type":"string","maxLength":200}}}),
            &["agent", "sessionId", "title", "cwd", "destination"],
        )
    }
    pub(super) fn validate(repo: &Repository, intent: &'a Value, attach: bool) -> Result<Self> {
        if !attach {
            let state = repo.state.lock().map_err(|_| Error::io())?;
            super::chat_edit::reject_unchecked_association(&state, intent)?;
        }
        Ok(Self {
            intent,
            imported: resolve_urls(repo, intent, |url| repo.providers.resolve(url))?,
            attach,
        })
    }
}
impl Recipe for NewChat<'_> {
    fn admission(&self) -> Option<(String, f64)> {
        None
    }
    /// An attach returns the chat already saved for its conversation; a new chat
    /// refuses to save that conversation again.
    fn reusable(&self, s: &Value) -> Result<Option<Value>> {
        if self.attach {
            return Ok(sessions::find(s, |row| {
                row["agent"] == self.intent["agent"] && row["sessionId"] == self.intent["sessionId"]
            })
            .cloned());
        }
        super::chat_edit::reject_unchecked_association(s, self.intent)?;
        Ok(None)
    }
    fn build(&self, _: &Repository, s: &mut Value, _: &mut Option<Created>) -> Result<Value> {
        save_linked(s, self.intent, &self.imported, SaveIntent::Save)
    }
}
/// A chat for one work item or folder, or with `review`, a review of one GitHub PR.
pub(super) struct Quick<'a> {
    intent: &'a Value,
    agent: &'a str,
    review: bool,
    review_command: String,
}
impl<'a> Quick<'a> {
    pub(super) fn schema() -> Value {
        chat_preparation::schema(
            json!({"agent":agent_schema(),"itemId":{"type":"string","maxLength":200},"scopeSource":{"type":"string","maxLength":80},"scope":{"type":"string","maxLength":1000},"folderPath":{"type":"string","maxLength":4096},"newFolder":{"type":"string","maxLength":80},"rememberKey":{"type":"string","maxLength":1000},"chooseFolder":{"type":"boolean"}}),
            &["agent"],
        )
    }
    pub(super) fn validate(repo: &Repository, intent: &'a Value, review: bool) -> Result<Self> {
        let agent = checked_agent(intent)?;
        let review_command = text(
            &repo.settings.values(),
            if agent == "claude" {
                "claudeReviewCommand"
            } else {
                "codexReviewCommand"
            },
        )
        .to_owned();
        if review && review_command.is_empty() {
            return Err(Error::new(
                "Choose a review skill in Settings → Agents & workflows.",
            ));
        }
        Ok(Self {
            intent,
            agent,
            review,
            review_command,
        })
    }
    fn item(&self, s: &Value) -> Result<Value> {
        let p = self.intent;
        let mut item = domain::catalog(s)
            .get(text(p, "itemId"))
            .cloned()
            .unwrap_or(Value::Null);
        if !text(p, "itemId").is_empty() && item.is_null() {
            return Err(Error::new("Work item unavailable."));
        }
        if item.is_null() && p["scopeSource"] == "github" && !text(p, "scope").is_empty() {
            item = arr(&s["items"])
                .into_iter()
                .find(|i| i["source"] == "github" && repo_name(i) == text(p, "scope"))
                .unwrap_or(Value::Null);
        }
        if self.review && (item["source"] != "github" || item["kind"] != "Pull request") {
            return Err(Error::new("Choose a GitHub PR."));
        }
        Ok(item)
    }
    fn review_instruction(
        &self,
        repo: &Repository,
        item: &Value,
        pr_label: &str,
    ) -> Result<String> {
        let (agent, review_command) = (self.agent, self.review_command.as_str());
        let builtin = super::skills_catalog::review_skill(repo.settings.config_directory());
        if builtin["commands"][agent] == review_command {
            if builtin["available"] != true {
                return Err(Error::new(
                    "The portable review skill is unavailable. Check the profile configuration.",
                ));
            }
            return Ok(format!("Read the complete truffle-review skill at {} and follow it for this assigned PR review. Start after registering this conversation. Review only {} at {}. Verify the current head and target branch, use a matching review worktree, and save the four reviewer reports as a registered Markdown artifact. No review request ID was supplied; do not invent one.",json!(text(&builtin,"path")),pr_label,text(item,"url")));
        }
        let template_path = repo
            .settings
            .config_directory()
            .join(format!("workflows/pr-review-{agent}.md"));
        let template = String::from_utf8(
            context_kit::read_bounded(&template_path, 16000)
                .map_err(|_| Error::new("Cannot read the review workflow template."))?,
        )
        .map_err(|_| Error::new("Review workflow template must be UTF-8."))?;
        Ok(review_skill_discovery(&template)
            .replace("{{REVIEW_COMMAND}}", review_command)
            .replace(
                "{{REVIEW_SKILL}}",
                review_command.trim_start_matches(['/', '$']),
            )
            .replace("{{PR_URL}}", text(item, "url"))
            .replace("{{PR_LABEL}}", pr_label)
            .replace("{{HOME}}", &home().to_string_lossy()))
    }
}
impl Recipe for Quick<'_> {
    fn admission(&self) -> Option<(String, f64)> {
        let key = if self.review {
            "pr-review-prepare"
        } else {
            "quick-session"
        };
        Some((key.into(), 3.0))
    }
    /// Fails for an unavailable work item. A review returns the newest unarchived
    /// review of the same PR by the same agent, also after it was opened.
    fn reusable(&self, s: &Value) -> Result<Option<Value>> {
        let item = self.item(s)?;
        if !self.review {
            return Ok(None);
        }
        Ok(sessions::latest(s, |x| {
            x["workflow"] == "pr-review"
                && x["reviewItemId"] == item["id"]
                && x["agent"] == self.agent
                && x["archivedAt"].is_null()
        })
        .cloned())
    }
    fn build(
        &self,
        repo: &Repository,
        s: &mut Value,
        created: &mut Option<Created>,
    ) -> Result<Value> {
        let (p, agent) = (self.intent, self.agent);
        let item = self.item(s)?;
        let root = work_root();
        let (cwd, new) = choose_with(s, &item, p, &root, &folders_in(&root))?;
        let choices = contexts(&item);
        let remember = p["rememberKey"]
            .as_str()
            .unwrap_or_else(|| choices.first().map_or("", |x| text(x, "key")));
        if !remember.is_empty() && !choices.iter().any(|x| x["key"] == remember) {
            return Err(Error::new("Choose a valid remembered folder scope."));
        }
        let pr_label = format!(
            "{}{}",
            repo_name(&item).split('/').next_back().unwrap_or(""),
            text(&item, "reference")
        );
        let title = if self.review {
            format!("Review {pr_label}")
        } else if !text(p, "itemId").is_empty() {
            text(&item, "title").chars().take(180).collect()
        } else {
            format!("New {agent} session")
        };
        let instruction = if self.review {
            self.review_instruction(repo, &item, &pr_label)?
        } else {
            String::new()
        };
        let ids = if text(p, "itemId").is_empty() {
            json!([])
        } else {
            json!([item["id"]])
        };
        let payload = json!({"agent":agent,"title":title,"cwd":cwd,"destination":"embedded","instruction":instruction,"itemIds":ids});
        sessions::validate_text(&payload)?;
        if new.is_some() {
            std::fs::create_dir_all(&root)?;
            chat_preparation::create_folder(created, std::path::Path::new(&cwd))?;
        }
        let record = sessions::save_with(s, &payload, |record| {
            if !text(p, "itemId").is_empty() {
                record["originItemId"] = item["id"].clone();
            }
            if self.review {
                record["workflow"] = json!("pr-review");
                record["reviewItemId"] = item["id"].clone();
            }
        })?;
        if !text(p, "folderPath").is_empty() || new.is_some() {
            if !remember.is_empty() {
                s["folderMappings"][remember] = json!({"path":cwd,"label":choices.iter().find(|x|x["key"]==remember).map_or(Value::Null, |x|x["label"].clone())});
            }
            if remember == format!("item:{}", text(&item, "id")) {
                s["itemSettings"][text(&item, "id")]["explicit"] = json!(true);
            }
        }
        Ok(record)
    }
}
/// A chat that reviews all work.
pub(super) struct WorkReview<'a> {
    agent: &'a str,
}
impl<'a> WorkReview<'a> {
    pub(super) fn schema() -> Value {
        chat_preparation::schema(json!({"agent":agent_schema()}), &["agent"])
    }
    pub(super) fn validate(intent: &'a Value) -> Result<Self> {
        Ok(Self {
            agent: checked_agent(intent)?,
        })
    }
}
impl Recipe for WorkReview<'_> {
    fn admission(&self) -> Option<(String, f64)> {
        Some(("work-review".into(), 30.0))
    }
    /// The newest unarchived work review by the same agent that was never opened.
    fn reusable(&self, s: &Value) -> Result<Option<Value>> {
        Ok(sessions::latest(s, |x| {
            x["workflow"] == "work-review"
                && x["agent"] == self.agent
                && x["launchNew"] == true
                && x["archivedAt"].is_null()
        })
        .cloned())
    }
    fn build(&self, repo: &Repository, s: &mut Value, _: &mut Option<Created>) -> Result<Value> {
        let mut times = json!({});
        for (source, info) in s["sources"].as_object().unwrap() {
            times[source] = info["lastSuccess"].clone();
        }
        sessions::save_with(
            s,
            &json!({"agent":self.agent,"title":"Work review","cwd":repo.root,"destination":"embedded","instruction":include_str!("../../resources/work-review.md"),"itemIds":[]}),
            |record| {
                record["workflow"] = json!("work-review");
                record["reviewSourceTimes"] = times;
            },
        )
    }
}
/// A chat that runs a configured skill preset.
pub(super) struct Skill {
    preset: Value,
}
impl Skill {
    pub(super) fn schema() -> Value {
        chat_preparation::schema(
            json!({"presetId":{"type":"string","maxLength":80}}),
            &["presetId"],
        )
    }
    pub(super) fn validate(repo: &Repository, intent: &Value) -> Result<Self> {
        let preset = artifacts::presets(repo, &repo.state.lock().unwrap())?
            .into_iter()
            .find(|x| x["id"] == intent["presetId"] && x["available"] == true)
            .ok_or_else(|| Error::new("Newsletter skill or folder unavailable."))?;
        Ok(Self { preset })
    }
}
impl Recipe for Skill {
    fn admission(&self) -> Option<(String, f64)> {
        Some((format!("newsletter:{}", text(&self.preset, "id")), 30.0))
    }
    /// The newest unarchived chat for the same preset that was never opened.
    fn reusable(&self, s: &Value) -> Result<Option<Value>> {
        Ok(sessions::latest(s, |x| {
            x["newsletterId"] == self.preset["id"]
                && x["launchNew"] == true
                && x["archivedAt"].is_null()
        })
        .cloned())
    }
    fn build(&self, _: &Repository, s: &mut Value, _: &mut Option<Created>) -> Result<Value> {
        let preset = &self.preset;
        let instruction=format!("Run the assigned skill now after registration. Read the complete skill at {} and follow {} in {}. Follow the skill's constraints. Save final reports under {} and register final outputs with the artifact helper using --skill-output. Register supporting attachments without --skill-output. This is a local dashboard run: do not send Slack messages or publish externally unless the user asks in this session. Do not wait for another instruction.",text(preset,"skill"),text(preset,"command"),text(preset,"cwd"),text(preset,"reports"));
        sessions::save_with(
            s,
            &json!({"agent":preset["agent"],"title":format!("{} · {}",text(preset,"title"),&super::repository::iso_time()[..10]),"cwd":preset["cwd"],"destination":"embedded","instruction":instruction,"itemIds":[]}),
            |record| {
                record["workflow"] = json!("newsletter");
                record["newsletterId"] = preset["id"].clone();
            },
        )
    }
}
/// A new chat with the context of the saved chat `intent.id`.
pub(super) struct Recover<'a> {
    pub intent: &'a Value,
}
impl Recover<'_> {
    pub(super) fn schema() -> Value {
        chat_preparation::schema(
            json!({"id":{"type":"string","maxLength":80},"cwd":{"type":"string","maxLength":4096}}),
            &["id"],
        )
    }
}
impl Recipe for Recover<'_> {
    fn admission(&self) -> Option<(String, f64)> {
        None
    }
    /// The newest recovery of the same chat that was never opened.
    fn reusable(&self, s: &Value) -> Result<Option<Value>> {
        let original = sessions::get(s, text(self.intent, "id"))?;
        Ok(sessions::latest(s, |x| {
            x["recoveryOf"] == original["id"] && x["launchNew"] == true
        })
        .cloned())
    }
    fn build(&self, _: &Repository, s: &mut Value, _: &mut Option<Created>) -> Result<Value> {
        let p = self.intent;
        let original = sessions::get(s, text(p, "id"))?.clone();
        let mut payload = original.clone();
        payload.as_object_mut().unwrap().remove("id");
        payload.as_object_mut().unwrap().remove("sessionId");
        if !text(p, "cwd").is_empty() {
            payload["cwd"] = p["cwd"].clone();
        }
        payload["itemIds"] = json!(arr(&original["items"])
            .iter()
            .map(|i| i["id"].clone())
            .collect::<Vec<_>>());
        sessions::save_with(s, &payload, |record| {
            for k in [
                "workflow",
                "reviewItemId",
                "newsletterId",
                "reviewSourceTimes",
            ] {
                record[k] = original[k].clone();
            }
            record["recoveryOf"] = original["id"].clone();
        })
    }
}
/// Git metadata is read only during explicit registration, never during UI reads.
pub(super) fn worktree_metadata(cwd: &str) -> Option<Value> {
    let output = terminal::run(
        "git",
        &[
            "-C".into(),
            cwd.into(),
            "rev-parse".into(),
            "--path-format=absolute".into(),
            "--show-toplevel".into(),
            "--git-common-dir".into(),
            "--abbrev-ref".into(),
            "HEAD".into(),
        ],
        None,
        3,
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
#[derive(Clone, Copy, PartialEq)]
enum FolderIntent {
    Replace,
    PreserveMapped,
}
pub(super) fn save_worktree(
    s: &mut Value,
    id: &str,
    metadata: &Value,
    ids: &[Value],
    primary: bool,
) -> Result<Value> {
    save_worktree_with_folders(s, id, metadata, ids, primary, FolderIntent::Replace)
}
fn save_worktree_with_folders(
    s: &mut Value,
    id: &str,
    metadata: &Value,
    ids: &[Value],
    primary: bool,
    folder_intent: FolderIntent,
) -> Result<Value> {
    let record = sessions::get(s, id)?.clone();
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
    entry["primary"] = json!(primary || old["primary"] == true);
    if primary {
        for tree in &mut trees {
            tree["primary"] = json!(false);
        }
    }
    trees.retain(|x| x["path"] != metadata["path"]);
    if trees.len() >= 64 {
        return Err(Error::new("A session can link at most 64 worktrees."));
    }
    trees.push(entry);
    let row = sessions::get_mut(s, id)?;
    row["worktrees"] = json!(trees);
    if primary {
        row["cwd"] = metadata["path"].clone();
    }
    row["updatedAt"] = json!(now());
    for id in ids {
        let key = id
            .as_str()
            .ok_or_else(|| Error::new("Invalid work item ID."))?;
        if folder_intent == FolderIntent::PreserveMapped
            && !text(&s["itemSettings"][key], "cwd").is_empty()
            && s["itemSettings"][key]["cwd"] != metadata["path"]
        {
            continue;
        }
        if !s["itemSettings"][key].is_object() {
            s["itemSettings"][key] = json!({});
        }
        s["itemSettings"][key]["cwd"] = metadata["path"].clone();
        s["itemSettings"][key]["explicit"] = json!(true);
    }
    Ok(sessions::get(s, id)?.clone())
}
pub fn registration_reply(record: &Value) -> Value {
    if !record["commandResult"].is_null() {
        return json!({"commandResult":record["commandResult"]});
    }
    if record["settings"].is_object() {
        return json!({"settings":record["settings"]});
    }
    let mut reply = json!({"id":record["id"],"linkedItems":arr(&record["items"]).len()});
    if record["artifactReceipt"].is_object() {
        reply["artifactReceipt"] = record["artifactReceipt"].clone();
    }
    if record["review"].is_object() {
        reply["review"] = record["review"].clone();
    }
    if record["context"].is_object() {
        reply["context"] = record["context"].clone();
    }
    reply
}
pub fn register(repo: &Arc<Repository>, p: &Value) -> Result<Value> {
    register_resolving(repo, p, |url| repo.providers.resolve(url))
}
fn register_review(repo: &Repository, p: &Value) -> Result<Value> {
    if let Some(report) = p.get("report") {
        if !report.is_object() || !report["comments"].is_array() {
            return Err(Error::new("Review report must contain a comments array."));
        }
        if report.get("overall").is_some() && !report["overall"].is_object() {
            return Err(Error::new("Overall assessment must be an object."));
        }
        if arr(&report["comments"]).is_empty()
            && !report["overall"].is_object()
            && text(p, "state").is_empty()
        {
            return Err(Error::new("The review batch has no comments."));
        }
    }
    let mut request = p.clone();
    request["action"] = json!(if !arr(&p["report"]["comments"]).is_empty()
        || p["report"]["overall"].is_object()
    {
        "append"
    } else if !text(p, "state").is_empty() {
        "status"
    } else {
        "read"
    });
    request["comments"] = p["report"]["comments"].clone();
    if let Some(overall) = p["report"].get("overall") {
        request["overall"] = overall.clone();
    }
    super::reviews::action(repo, &request)
}
fn register_worktree(repo: &Repository, p: &Value) -> Result<Value> {
    let path = directory(text(p, "path"))?;
    let metadata = worktree_metadata(&path).ok_or_else(|| {
        Error::new("Choose an existing Git repository or worktree with a commit.")
    })?;
    let record = sessions::get(&repo.state.lock().unwrap(), text(p, "id"))?.clone();
    let mut ids = arr(&p["itemIds"]);
    let urls = arr(&p["urls"]);
    if ids.len() + urls.len() > 100 {
        return Err(Error::new("Link at most 100 work items at once."));
    }
    for url in &urls {
        safe_url(
            url.as_str()
                .ok_or_else(|| Error::new("Invalid work URL."))?,
        )?;
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
    if ids.is_empty() && urls.is_empty() {
        ids = arr(&record["items"])
            .iter()
            .map(|x| x["id"].clone())
            .collect();
    }
    repo.transaction(|s| save_worktree(s, text(p, "id"), &metadata, &ids, p["primary"] == true))
}
fn apply_background_activity(
    runtime: &mut Value,
    old: &Value,
    p: &Value,
    has_subagents: bool,
) -> Result<()> {
    // Turn-end hooks carry an authoritative snapshot of background work.
    // Omitted snapshots mean unknown, not zero; unrelated events retain it.
    if p["event"] != "SessionStart" {
        for field in ["backgroundCounts", "scheduledWakeups", "activityAt"] {
            if !old[field].is_null() {
                runtime[field] = old[field].clone();
            }
        }
    }
    if let Some(counts) = p.get("backgroundCounts") {
        let counts = counts
            .as_object()
            .ok_or_else(|| Error::new("Invalid background activity counts."))?;
        if counts.len() > 100
            || counts.iter().any(|(kind, count)| {
                kind.is_empty()
                    || kind.chars().count() > 40
                    || count.as_u64().is_none_or(|n| n > 1000)
            })
        {
            return Err(Error::new("Invalid background activity counts."));
        }
        runtime["backgroundCounts"] = json!(counts);
        runtime["activityAt"] = json!(now());
    }
    if let Some(count) = p.get("scheduledWakeups") {
        if count.as_u64().is_none_or(|n| n > 1000) {
            return Err(Error::new("Invalid scheduled wakeup count."));
        }
        runtime["scheduledWakeups"] = count.clone();
        runtime["activityAt"] = json!(now());
    }
    let background = runtime["backgroundCounts"]
        .as_object()
        .map_or(0, |v| v.values().filter_map(Value::as_u64).sum::<u64>());
    if runtime["state"] == "waiting"
        && (background > 0
            || has_subagents
            || runtime["scheduledWakeups"].as_u64().unwrap_or(0) > 0)
    {
        runtime["state"] = json!("background");
    } else if runtime["state"] == "background"
        && background == 0
        && !has_subagents
        && runtime["scheduledWakeups"].as_u64().unwrap_or(0) == 0
    {
        runtime["state"] = json!("waiting");
    }
    Ok(())
}
fn complete_skill_runs(s: &mut Value, session_id: &Value) {
    for run in s["skillRuns"].as_array_mut().unwrap() {
        if run["sessionId"] == *session_id
            && ["starting", "launched"].contains(&text(run, "outcome"))
        {
            run["outcome"] = json!("completed");
            run["finishedAt"] = json!(now());
            run["message"] = json!("Agent finished its turn. Registered results appear below.");
        }
    }
}
fn apply_runtime_event(s: &mut Value, p: &Value) -> Result<Value> {
    let row = sessions::get_mut(s, text(p, "id"))?;
    if p["event"] == "SessionStart" && !text(p, "sessionId").is_empty() {
        let id = sid(text(p, "sessionId"))?;
        if text(row, "sessionId").is_empty() {
            row["sessionId"] = json!(id);
        }
    }
    let old = row["runtime"].clone();
    let mut ids = arr(&old["subagentIds"]);
    if p["event"] == "SessionStart" {
        ids.clear();
    }
    let sub = checked(p, "subagentId", 200, false)?;
    let state = match text(p, "event") {
        "SessionStart" | "Stop" => "waiting",
        "UserPromptSubmit" => "working",
        "PermissionRequest" => "needs-input",
        "SubagentStart" => {
            if !sub.is_empty() && !ids.contains(&json!(sub)) && ids.len() < 1000 {
                ids.push(json!(sub));
            }
            old["state"].as_str().unwrap_or("working")
        }
        "SubagentStop" => {
            ids.retain(|x| x != &json!(sub));
            old["state"].as_str().unwrap_or("working")
        }
        _ => return Err(Error::new("Unsupported runtime event.")),
    };
    let reason = match text(p, "event") {
        "PermissionRequest" => "approval",
        "Stop" => "finished",
        "SessionStart" => "ready",
        "UserPromptSubmit" => "",
        _ => text(&old, "reason"),
    };
    let mut runtime =
        json!({"reason":reason,"state":state,"subagents":ids.len(),"subagentIds":ids,"at":now()});
    apply_background_activity(&mut runtime, &old, p, !ids.is_empty())?;
    row["runtime"] = runtime;
    let record = row.clone();
    if ["Stop", "SubagentStop"].contains(&text(p, "event"))
        && record["runtime"]["state"] == "waiting"
        && record["runtime"]["subagents"] == 0
    {
        complete_skill_runs(s, &record["id"]);
    }
    Ok(record)
}
fn register_runtime(repo: &Repository, p: &Value) -> Result<Value> {
    let state = checked(p, "state", 32, true)?;
    if !["working", "waiting", "needs-input"].contains(&state.as_str()) {
        return Err(Error::new("Choose working, waiting, or needs-input."));
    }
    if p.get("subagents")
        .is_some_and(|v| v.as_u64().is_none_or(|x| x > 1000))
    {
        return Err(Error::new("Subagent count must be 0–1000."));
    }
    repo.transaction(|s| {
        let row = sessions::get_mut(s, text(p, "id"))?;
        let reason = checked(p, "reason", 32, false)?;
        if !["", "ready", "finished", "question", "approval", "failed"].contains(&reason.as_str()) {
            return Err(Error::new("Invalid attention reason."));
        }
        row["runtime"] =
            json!({"state":state,"reason":reason,"subagents":p["subagents"],"at":now()});
        Ok(row.clone())
    })
}
fn save_session_output(repo: &Repository, p: &Value) -> Result<Value> {
    let record = sessions::get(&repo.state.lock().unwrap(), text(p, "id"))?.clone();
    if p["operation"] == "save-artifact" {
        let receipt = super::artifacts::attach_to_chat(repo, text(&record, "id"), p)?;
        let mut reply = record;
        reply["artifactReceipt"] = receipt;
        return Ok(reply);
    }
    let mut edit = p.clone();
    edit["action"] = p["operation"].clone();
    repo.edit(&edit)?;
    Ok(record)
}
const AGENT_COMMAND_LOG: usize = 20;
/// Run an app command for an agent. Writes and user-action commands are logged on the
/// session first, so its chat shows every one that ran.
fn agent_command(repo: &Arc<Repository>, p: &Value) -> Result<Value> {
    let name = text(p, "name");
    if !commands::shown_to_user(name) {
        return Ok(json!({"commandResult": commands::invoke(repo, p, false)?}));
    }
    let id = sid(text(p, "id"))?;
    let entry = json!({"id": uuid(), "name": name, "at": now()});
    repo.transaction(|s| {
        let record = sessions::get_mut(s, &id).map_err(|_| {
            Error::new("Pass this chat's --record ID so the chat can show the command.")
        })?;
        let mut log = arr(&record["agentCommands"]);
        log.push(entry.clone());
        let excess = log.len().saturating_sub(AGENT_COMMAND_LOG);
        log.drain(..excess);
        record["agentCommands"] = Value::Array(log);
        Ok(())
    })?;
    let result = commands::invoke(repo, p, false);
    let ok = result.is_ok();
    // The command already ran; failing to store its outcome must not report it as failed.
    let _ = repo.transaction(|s| {
        if let Some(logged) = sessions::get_mut(s, &id)
            .ok()
            .and_then(|record| record["agentCommands"].as_array_mut())
            .and_then(|log| log.iter_mut().find(|row| row["id"] == entry["id"]))
        {
            logged["ok"] = json!(ok);
        }
        Ok(())
    });
    Ok(json!({"commandResult": result?}))
}
fn register_resolving(
    repo: &Arc<Repository>,
    p: &Value,
    resolve: impl Fn(&str) -> Result<Value>,
) -> Result<Value> {
    if p["operation"] == "command" {
        return agent_command(repo, p);
    }
    if p["operation"] == "settings-read" {
        return Ok(json!({"settings":repo.settings.view()}));
    }
    if p["operation"] == "settings-apply" {
        return Ok(json!({"settings":repo.settings.apply(p,true)?}));
    }
    if p["operation"] == "ci-investigation" {
        return super::session_ci::investigation(repo, p);
    }
    if p["operation"] == "review" {
        return register_review(repo, p);
    }
    if p["operation"] == "context-read" {
        return super::session_context::read(repo, text(p, "id"));
    }
    if p["operation"] == "context-replace" {
        return super::session_context::replace(repo, text(p, "id"), &p["context"]);
    }
    if p["operation"] == "resources" {
        return super::resources::apply(repo, text(p, "id"), &p["report"]);
    }
    if p["operation"] == "worktree" {
        return register_worktree(repo, p);
    }

    if p["operation"] == "runtime-event" {
        return repo.transaction(|s| apply_runtime_event(s, p));
    }
    if p["operation"] == "runtime" {
        return register_runtime(repo, p);
    }
    if ["checkpoint", "save-briefing", "save-artifact"].contains(&text(p, "operation")) {
        return save_session_output(repo, p);
    }
    sid(text(p, "sessionId"))?;
    let requested_ids = arr(&p["itemIds"]);
    let requested_urls = arr(&p["urls"]);
    let mut p = p.clone();
    p["preserveItemFolders"] = json!(true);
    p["sessionId"] = json!(sid(text(&p, "sessionId"))?);
    let imported = resolve_urls(repo, &p, resolve)?;
    let record = repo.transaction(|s| save_linked(s, &p, &imported, SaveIntent::Register))?;
    let metadata = worktree_metadata(text(&record, "cwd"));
    finish_registration(
        repo,
        &record,
        &requested_ids,
        &requested_urls,
        metadata.as_ref(),
    )
}
fn finish_registration(
    repo: &Repository,
    record: &Value,
    requested_ids: &[Value],
    requested_urls: &[Value],
    metadata: Option<&Value>,
) -> Result<Value> {
    let ids = arr(&record["items"])
        .iter()
        .filter(|x| {
            requested_ids.is_empty() && requested_urls.is_empty()
                || requested_ids.contains(&x["id"])
                || requested_urls.contains(&x["url"])
        })
        .map(|x| x["id"].clone())
        .collect::<Vec<_>>();
    repo.transaction(|s| {
        let latest = sessions::get(s, text(record, "id"))?.clone();
        // Git metadata belongs to the probed folder, not a later folder edit.
        if latest["cwd"] == record["cwd"] {
            let current_ids = ids
                .iter()
                .filter(|id| arr(&latest["items"]).iter().any(|item| item["id"] == **id))
                .cloned()
                .collect::<Vec<_>>();
            if let Some(metadata) = metadata {
                save_worktree_with_folders(
                    s,
                    text(record, "id"),
                    metadata,
                    &current_ids,
                    true,
                    FolderIntent::PreserveMapped,
                )?;
            } else {
                for id in &current_ids {
                    let key = id.as_str().unwrap();
                    if text(&s["itemSettings"][key], "cwd").is_empty() {
                        if !s["itemSettings"][key].is_object() {
                            s["itemSettings"][key] = json!({});
                        }
                        s["itemSettings"][key]["cwd"] = latest["cwd"].clone();
                    }
                }
            }
        }
        let row = sessions::get_mut(s, text(record, "id"))?;
        row["launchNew"] = json!(false);
        Ok(row.clone())
    })
}
pub fn context(repo: &Repository, id: &str) -> Result<String> {
    let snapshot = repo.state.lock().map_err(|_| Error::io())?.clone();
    context_from_state(repo, id, &snapshot)
}
fn context_from_state(repo: &Repository, id: &str, s: &Value) -> Result<String> {
    let record = sessions::get(s, id)?;
    let helper = super::agent_cli::helper(&repo.root)?;
    let register = format!(
        "{helper} register --record {} --agent {} --title {} --cwd {}{}",
        sid(id)?,
        text(record, "agent"),
        terminal::quote(text(record, "title")),
        terminal::quote(text(record, "cwd")),
        if text(record, "sessionId").is_empty() {
            String::new()
        } else {
            format!(" --session {}", sid(text(record, "sessionId"))?)
        }
    );
    let mut sections=vec![format!("# {}\n\n## Your instruction\n{}\n\n## Working directory\n{}\n\n## Dashboard registration\nBefore your first reply, register this conversation with truffle. Do not create a placeholder project.\n{register}\nThe helper reads CODEX_THREAD_ID or CLAUDE_CODE_SESSION_ID. If unavailable, ask for the exact UUID. Do not guess.\nRegister again with the same --record and --url when related work is supplied or created. Keep existing links. Register the actual selected worktree cwd.\nDo not start or resume another conversation to register.\n\n## Handoff notes\nSave a short checkpoint JSON with done, blocked, and next.\n{helper} checkpoint --record {id} --file /absolute/path/checkpoint.json\n\n## Save artifacts\nAttach each final display output; this does not upload the file.\n{helper} artifact --record {id} --file /absolute/path/report.md --title 'Review report'\nUse --url for an existing HTTPS artifact. Never invent a URL. Prefer saving reports as Markdown, tables as UTF-8 CSV with headers (or truffle.table/v1 JSON), and plots as PNG/WebP. Python scripts can be attached directly as .py artifacts for a read-only code preview; do not wrap them in Markdown merely to attach them. Register every finished result so it renders in the agent workspace; do not make the user read large tables in the terminal. Local outputs are snapshotted.\n",text(record,"title"),text(record,"instruction"),text(record,"cwd"))];
    sections.push(format!("\n## Worktree reporting\nRead {} for the truffle reporting protocol. After creating, selecting, or switching a worktree, immediately re-run registration with this same record/conversation UUID and --cwd set to the actual worktree. Include its ticket/PR URLs. This records the branch, main repository, worktree and work-item associations and makes Continue use that directory. Reuse an existing matching worktree under ~/code/<repo> before creating ~/code/<repo>/.worktrees/<task>. Do not move existing worktrees. For additional worktrees in this conversation, register their work URLs first, then use: {helper} worktree --record {id} --path /absolute/worktree --url https://verified-work-url [--primary]. Omit --primary for secondary worktrees; it changes the resume directory when supplied. Do not guess links or create worktrees just for registration.\n",repo.data.parent().unwrap().join("runtime-docs/truffle.md").display()));
    sections.push(format!("\n## Resource reporting\nWhen you create, reuse, resize, stop or learn the status of a VM, GPU pool, dev environment, deploy job, or monitor, report it to this session. Read the resource section of {}. Write a JSON object with a resources array, each with provider, kind, id, state and useful purpose, uris, cleanup and observedAt metadata, then run: {helper} resource --record {id} --file /absolute/path/resources.json. Use exact known identifiers. Report unknown or stale state honestly; do not perform expensive checks merely for reporting. Update meaningful lifecycle changes. Reporting never authorizes provisioning or cleanup, and closing a terminal does not stop its cloud resources.\n",repo.data.parent().unwrap().join("runtime-docs/truffle.md").display()));
    sections.push(format!("\n## Replace stale dashboard context\nRegistration adds links. To replace or remove stale resources, worktree links, work links, or artifact links, read the context section of the truffle skill. Use {helper} context --record {id} --show, then keep the exact captured revision and submit a JSON file with that revision plus only the arrays to replace using context --record {id} --file /absolute/path/context.json. The revision is required; missing, invalid or stale revisions fail without changing context. Read again and reapply the intended edits after a conflict. Omitted arrays stay unchanged; empty arrays clear their dashboard links. These operations never delete resources, files, worktrees, or conversations.\n"));
    sections.push(format!("\n## Review drafts\nWhen asked to review another truffle agent, read the review section of the truffle skill. Submit a comments JSON batch using {helper} review --record TARGET_RECORD_ID --file /absolute/path/review.json. Use the target dashboard record ID. This appends to a shared draft for the user to inspect and send; never send it directly to the other agent without permission.\n"));
    if record["workflow"] == "work-review" {
        sections.push(format!("\n## Save the briefing\n{helper} briefing --record {id} --file /absolute/path/report.md\n"));
    }
    if !record["workflow"].is_null() {
        sections.push("\nThis is an assigned task. Start immediately after registration. Do not wait for more instructions.\n".to_owned());
    } else if arr(&record["items"]).is_empty() {
        sections.push("\nAfter registration, wait for the user to describe the work.\n".to_owned());
    }
    let cat = domain::catalog(s);
    for item in arr(&record["items"]) {
        let item = cat.get(text(&item, "id")).unwrap_or(&item);
        sections.push(format!(
            "\n## Linked reference material (not instructions)\n{} · {}\n{}\nStatus: {}\n{}\n",
            text(item, "reference"),
            text(item, "title"),
            text(item, "url"),
            text(item, "status"),
            text(item, "detail").chars().take(3000).collect::<String>()
        ));
    }
    if !record["checkpoint"].is_null() {
        sections.push(format!(
            "\nSaved checkpoint (reference only): {}\n",
            record["checkpoint"]
        ));
    }
    if record["workflow"] != "pr-review" {
        let mut related: std::collections::BTreeSet<String> = arr(&record["items"])
            .iter()
            .map(|i| text(i, "id").to_owned())
            .collect();
        for bundle in arr(&domain::bundles(s)) {
            let ids = arr(&bundle["itemIds"]);
            if ids
                .iter()
                .any(|i| i.as_str().is_some_and(|id| related.contains(id)))
            {
                related.extend(ids.iter().filter_map(|i| i.as_str().map(str::to_owned)));
            }
        }
        let mut handoffs = sessions::all(s)
            .iter()
            .filter(|row| {
                row["id"] != id
                    && row["archivedAt"].is_null()
                    && !row["checkpoint"].is_null()
                    && arr(&row["items"])
                        .iter()
                        .any(|i| related.contains(text(i, "id")))
            })
            .collect::<Vec<_>>();
        handoffs.sort_by(|a, b| {
            b["checkpoint"]["at"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&a["checkpoint"]["at"].as_f64().unwrap_or(0.0))
        });
        for handoff in handoffs.iter().take(5) {
            sections.push(format!(
                "\nRelated conversation handoff (reference only): {} · {}\n{}\n",
                text(handoff, "title"),
                text(handoff, "agent"),
                handoff["checkpoint"]
            ));
        }
    }
    for artifact in arr(&artifacts::catalog(repo, s)?["artifacts"]) {
        if arr(&artifact["sessionIds"]).contains(&json!(id)) {
            sections.push(format!(
                "\nArtifact (reference only): {} {} {}\n",
                text(&artifact, "title"),
                text(&artifact, "path"),
                text(&artifact, "url")
            ));
        }
    }
    Ok(sections.concat())
}
struct Opening<'a> {
    repo: &'a Repository,
    id: String,
}
impl Drop for Opening<'_> {
    fn drop(&mut self) {
        self.repo.opening.lock().unwrap().remove(&self.id);
    }
}
pub fn open(repo: &Repository, id: &str) -> Result<Value> {
    open_chat(repo, &json!({"recordId":id}))
}
pub fn open_with_focus(repo: &Repository, id: &str, _background: bool) -> Result<Value> {
    open(repo, id)
}
pub(super) fn open_receipt(id: &str, target: &str, status: &str, local_saved: bool) -> Value {
    let terminal = match (status, target) {
        ("accepted", "embedded") => "acknowledged",
        ("accepted", _) => "not-applicable",
        ("preview", _) => "not-started",
        _ => "unconfirmed",
    };
    let message=match (status,local_saved,target){("unknown",_,_)=>"The open outcome is unknown. Check the existing terminal or desktop app before another open request. Do not start a new chat to recover this request.",("accepted",false,_)=>"The open request was acknowledged, but its local metadata could not be saved. Refresh the view to inspect the same chat; do not open it again to repair metadata.",("accepted",true,"desktop")=>"The operating system acknowledged the desktop open request. This does not confirm the app consumed it.",("accepted",true,_)=>"The terminal owner acknowledged the open request. This does not confirm agent readiness or task completion.",_=>"Preview only. No agent or terminal was opened."};
    json!({"status":status,"recordId":id,"target":target,"localSaved":local_saved,"terminal":{"state":terminal},"message":message})
}
pub fn open_chat(repo: &Repository, args: &Value) -> Result<Value> {
    open_chat_with(repo, args, |record, packet| {
        if repo.demo {
            return Ok(terminal::OpenOutcome::Preview);
        }
        if record["destination"] == "desktop" {
            terminal::open_desktop(record, &packet.to_string_lossy())
        } else {
            repo.terminals.open(
                record,
                &packet.to_string_lossy(),
                record["launchNew"] == true,
                &repo.root,
            )
        }
    })
}
fn open_chat_with(
    repo: &Repository,
    args: &Value,
    launch: impl FnOnce(&Value, &std::path::Path) -> Result<terminal::OpenOutcome>,
) -> Result<Value> {
    open_chat_observing(repo, args, launch, |record| {
        repo.terminals
            .inventory()
            .map_or(terminal::RuntimeObservation::Unavailable, |inventory| {
                terminal::existing_codex_runtime(record, inventory.rows(), false)
            })
    })
}
fn open_chat_arguments<'a>(repo: &Repository, args: &'a Value) -> Result<(String, &'a str)> {
    let fields = args
        .as_object()
        .ok_or_else(|| Error::new("Open arguments must be an object."))?;
    if fields
        .keys()
        .any(|key| !["recordId", "target"].contains(&key.as_str()))
    {
        return Err(Error::new(
            "Open accepts only recordId and a destination intent.",
        ));
    }
    let id = sid(args["recordId"]
        .as_str()
        .ok_or_else(|| Error::new("Supply the saved chat ID."))?)?;
    let intent = match args.get("target") {
        None => "saved",
        Some(value) => value
            .as_str()
            .filter(|value| ["saved", "embedded"].contains(value))
            .ok_or_else(|| Error::new("Choose the saved destination or embedded terminal."))?,
    };
    for value in [
        &id,
        "accepted",
        "preview",
        "unknown",
        "embedded",
        "desktop",
        "acknowledged",
        "observed",
        "not-applicable",
        "not-started",
        "unconfirmed",
    ] {
        let mut projected = value.to_owned();
        commands::redact_text(repo, &mut projected);
        if projected != value {
            return Err(Error::new(
                "The saved chat open receipt cannot be safely projected.",
            ));
        }
    }
    Ok((id, intent))
}
fn open_unregistered_codex(
    repo: &Repository,
    id: &str,
    intent: &str,
    observation: terminal::RuntimeObservation,
) -> Result<Value> {
    match observation {
        terminal::RuntimeObservation::Observed(generation)=>{
            let eligible=sessions::get(&*repo.state.lock().map_err(|_|Error::io())?,id).is_ok_and(|row|row["agent"]=="codex"&&row["launchNew"]==false&&(row["sessionId"].is_null()||row["sessionId"]=="")&&(intent=="embedded"||row["destination"]!="desktop"));
            if !eligible{return Ok(open_receipt(id,"embedded","unknown",false));}
            let mut projected=generation.clone();commands::redact_text(repo,&mut projected);if projected!=generation{return Err(Error::new("Terminal generation cannot be safely projected."));}let mut receipt=open_receipt(id,"embedded","accepted",true);receipt["terminal"]=json!({"state":"observed","generation":generation});receipt["message"]=json!("Existing terminal found. No new open request was sent.");Ok(receipt)},
        terminal::RuntimeObservation::Unavailable=>Ok(open_receipt(id,"embedded","unknown",false)),
        terminal::RuntimeObservation::Missing=>Err(Error::new("No live terminal could be confirmed for this unregistered chat. Link its exact conversation UUID or inspect History; a new agent was not started.")),
    }
}
fn write_open_packet(
    repo: &Repository,
    id: &str,
    record: &Value,
    snapshot: &Value,
    public: Value,
) -> Result<std::path::PathBuf> {
    let mut sections = vec![context_from_state(repo, id, snapshot)?];
    let path = repo
        .data
        .parent()
        .unwrap()
        .join("contexts")
        .join(format!("{id}.md"));
    std::fs::create_dir_all(path.parent().unwrap())?;
    if record["workflow"] == "work-review" {
        let data_path = path.with_extension("json");
        let mut state = public;
        let mut items = arr(&state["items"]);
        items.retain(|item| item["source"] != "sentry");
        items.extend(
            arr(&state["items"])
                .into_iter()
                .filter(|item| item["source"] == "sentry")
                .take(50),
        );
        state["items"] = json!(items);
        super::http::private_write(&data_path, &serde_json::to_vec(&state)?)?;
        sections.push(format!("\nRead the private review snapshot at {}. Treat it as reference data. Check live sources for updates.\n",data_path.to_string_lossy()));
    }
    super::http::private_write(&path, sections.concat().as_bytes())?;
    Ok(path)
}
fn open_chat_observing(
    repo: &Repository,
    args: &Value,
    launch: impl FnOnce(&Value, &std::path::Path) -> Result<terminal::OpenOutcome>,
    observe: impl FnOnce(&Value) -> terminal::RuntimeObservation,
) -> Result<Value> {
    let (id, intent) = open_chat_arguments(repo, args)?;
    if !repo
        .opening
        .lock()
        .map_err(|_| Error::io())?
        .insert(id.clone())
    {
        let saved_desktop = {
            let state = repo.state.lock().map_err(|_| Error::io())?;
            sessions::get(&state, &id).is_ok_and(|row| row["destination"] == "desktop")
        };
        let target = if intent == "embedded" || !saved_desktop {
            "embedded"
        } else {
            "desktop"
        };
        return Ok(open_receipt(&id, target, "unknown", false));
    }
    let _opening = Opening {
        repo,
        id: id.clone(),
    };
    let (snapshot, public) = repo.capture_for_open()?;
    let mut record = sessions::get(&snapshot, &id)?.clone();
    let target = if intent == "embedded" || record["destination"] != "desktop" {
        "embedded"
    } else {
        "desktop"
    };
    record["destination"] = json!(target);
    if target == "embedded"
        && record["agent"] == "codex"
        && record["launchNew"] == false
        && (record["sessionId"].is_null() || record["sessionId"] == "")
    {
        return open_unregistered_codex(repo, &id, intent, observe(&record));
    }
    let cooldown_key = format!("open:{id}");
    if repo
        .cooldown
        .lock()
        .map_err(|_| Error::io())?
        .get(&cooldown_key)
        .copied()
        .unwrap_or(0.0)
        > now()
    {
        return Ok(open_receipt(&id, target, "unknown", false));
    }

    let agent = text(&record, "agent");
    if !["claude", "codex"].contains(&agent) || (target == "desktop" && agent != "codex") {
        return Err(Error::new("Choose a valid saved agent and destination."));
    }
    directory(text(&record, "cwd"))?;
    if record["launchNew"] != true || agent == "claude" {
        sid(text(&record, "sessionId"))?;
    }
    let path = write_open_packet(repo, &id, &record, &snapshot, public)?;
    let outcome = launch(&record, &path)?;
    // The per-record opening guard serializes admission. Only a completed
    // transport attempt consumes cooldown; validation and local I/O may be fixed
    // and retried immediately without creating an artificial unknown outcome.
    repo.cooldown
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(cooldown_key, now() + 3.0);
    if outcome == terminal::OpenOutcome::Unknown {
        return Ok(open_receipt(&id, target, "unknown", false));
    }
    let status = if outcome == terminal::OpenOutcome::Preview {
        "preview"
    } else {
        "accepted"
    };
    // Patch only launch bookkeeping in the latest canonical record. The captured
    // context is never replayed over newer metadata or work links.
    let saved = repo
        .transaction_if_changed(|state| {
            let row = sessions::get_mut(state, &id)?;
            row["launchNew"] = json!(false);
            row["lastOpenedAt"] = json!(now());
            if target == "embedded" && outcome == terminal::OpenOutcome::Accepted {
                row["destination"] = json!("embedded");
                row["workspaceId"] = Value::Null;
                row["surfaceId"] = Value::Null;
            }
            Ok(())
        })
        .is_ok();
    Ok(open_receipt(&id, target, status, saved))
}

/// Repair only the exact legacy stock discovery clauses at render time. Keep
/// user-authored workflow files and all unrelated template text unchanged.
pub fn review_skill_discovery(template: &str) -> String {
    template
        .replace(
            "Use the {{REVIEW_SKILL}} skill. Read its complete instructions at {{HOME}}/.agents/skills{{REVIEW_COMMAND}}/SKILL.md.",
            "Use the {{REVIEW_SKILL}} skill through this agent's native installed skill discovery. Read its complete current instructions from the actual resolved skill location, even if you loaded an older copy earlier. Respect project, user, configured, and plugin skill locations. If the skill is missing or ambiguous, report the blocker and stop; do not construct a path from the command name.",
        )
        .replace(
            "Read the current {{SKILL_NAME}} SKILL.md from disk at ~/.claude/skills/{{SKILL_NAME}}/SKILL.md (Claude) or ~/.agents/skills/{{SKILL_NAME}}/SKILL.md (Codex), even if you loaded an older copy earlier.",
            "Resolve {{REVIEW_COMMAND}} through this agent's native installed skill discovery and read the complete current {{SKILL_NAME}} skill from its actual resolved location, even if you loaded an older copy earlier. Respect project, user, configured, and plugin skill locations. If the skill is missing or ambiguous, report the blocker and stop; do not construct a path from the command name.",
        )
}

#[cfg(test)]
mod portable_review_tests;

#[cfg(test)]
mod folder_tests;

#[cfg(test)]
mod saved_chat_open_tests;

#[cfg(test)]
mod registration_merge_tests;

#[cfg(test)]
mod live_unregistered_open_tests;

#[cfg(test)]
mod agent_command_tests;
