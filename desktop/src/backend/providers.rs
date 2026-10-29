//! Provider adapters. Explicit PR actions are fixed mutations against cached identities.
use super::*;
use reqwest::{blocking::Client, redirect::Policy};
use std::collections::BTreeMap;
// Snapshot redaction for a single output stream. Never publish a possible secret
// prefix: the next read can complete it. Overlarge sensitive prefixes fail closed.
pub(super) struct StreamRedactor {
    secrets: Vec<String>,
    pending: String,
    suppressed: bool,
}
impl StreamRedactor {
    const MAX_PENDING: usize = 64 * 1024;
    pub(super) fn with_values(mut self, values: impl IntoIterator<Item=String>) -> Self {
        for value in values {
            if !value.is_empty() && !self.secrets.contains(&value) { self.secrets.push(value); }
        }
        self.secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
        self
    }
    pub(super) fn push(&mut self, text: &str) -> String {
        if self.suppressed { return String::new(); }
        self.pending.push_str(text);
        let mut output = String::new();
        let mut offset = 0;
        while offset < self.pending.len() {
            let remaining = &self.pending[offset..];
            if self.secrets.iter().any(|secret| secret.len() > remaining.len() && secret.starts_with(remaining)) {
                if remaining.len() >= Self::MAX_PENDING {
                    output.push_str("[Output omitted: sensitive value exceeds stream safety limit.]");
                    self.suppressed = true;
                    offset = self.pending.len();
                }
                break;
            }
            if let Some(secret) = self.secrets.iter().find(|secret| remaining.starts_with(secret.as_str())) {
                output.push_str("[redacted]");
                offset += secret.len();
            } else {
                let character = remaining.chars().next().unwrap();
                output.push(character);
                offset += character.len_utf8();
            }
        }
        self.pending.drain(..offset);
        output
    }
    pub(super) fn finish(&mut self) -> String {
        if self.pending.is_empty() { String::new() } else {
            self.pending.clear();
            "[redacted]".into()
        }
    }
}
#[derive(Clone)]
pub struct Providers {
    client: Client,
    pub sentry_org: String,
    pub sentry_period: String,
    sentry_base: String,
    sentry_token: String,
    linear_token: String,
    github_token: String,
    redaction_secrets: Vec<String>,
    fields: Value,
}
impl Providers {
    pub fn new(root: &std::path::Path) -> Result<Self> {
        let mut dotenv = BTreeMap::new();
        if let Ok(env) = std::fs::read_to_string(root.join(".env")) {
            for line in env.lines() {
                if let Some((key, value)) = line.trim().split_once('=') {
                    if !key.starts_with('#') {
                        dotenv.entry(key.to_owned()).or_insert_with(|| {
                            value.trim().trim_matches(['\'', '"']).to_owned()
                        });
                    }
                }
            }
        }
        let mut ini = BTreeMap::new();
        let mut section = String::new();
        if let Ok(config) = std::fs::read_to_string(home().join(".sentryclirc")) {
            for line in config.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    section = line.trim_matches(['[', ']']).into();
                } else if let Some((k, v)) = line.split_once('=') {
                    ini.insert(format!("{section}.{}", k.trim()), v.trim().to_string());
                }
            }
        }
        let setting = |env: &str, key: &str, default: &str| {
            std::env::var(env).unwrap_or_else(|_| {
                dotenv.get(env).cloned().unwrap_or_else(|| {
                    ini.get(key).cloned().unwrap_or(default.into())
                })
            })
        };
        let org = setting("SENTRY_ORG", "defaults.org", "");
        let base = setting("SENTRY_URL", "defaults.url", "https://sentry.io");
        let period = setting("SENTRY_STATS_PERIOD", "", "90d");
        if ![
            "https://sentry.io",
            "https://us.sentry.io",
            "https://de.sentry.io",
        ]
        .contains(&base.as_str())
            || !regex::Regex::new(r"^(?:[1-9]|[1-8][0-9]|90)d$")
                .unwrap()
                .is_match(&period)
            || !org
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
        {
            return Err(Error::new("Invalid Sentry cloud configuration."));
        }
        let sentry_token = setting("SENTRY_AUTH_TOKEN", "auth.token", "");
        let linear_token = setting("LINEAR_API_KEY", "", "");
        let github_token = setting("GITHUB_TOKEN", "", "");
        let mut redaction_secrets = dotenv.iter().filter_map(|(key, value)| {
            let key = key.to_ascii_uppercase();
            (key.contains("TOKEN") || key.contains("SECRET") || key.contains("PASSWORD")
                || key.ends_with("API_KEY")).then(|| value.clone())
        }).collect::<Vec<_>>();
        redaction_secrets.extend([
            sentry_token.clone(), linear_token.clone(), github_token.clone(),
            ini.get("auth.token").cloned().unwrap_or_default(),
        ]);
        redaction_secrets.retain(|value| !value.is_empty());
        redaction_secrets.sort();
        redaction_secrets.dedup();
        redaction_secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
        Ok(Self {
            client: Client::builder()
                .user_agent("mydash-local/1")
                .redirect(Policy::none())
                .timeout(std::time::Duration::from_secs(25))
                .build()
                .map_err(|_| Error::io())?,
            sentry_org: org,
            sentry_period: period,
            sentry_base: base,
            sentry_token,
            linear_token,
            github_token,
            redaction_secrets,
            fields: serde_json::from_str(include_str!("../../resources/provider-fields.json"))?,
        })
    }
    pub(super) fn configure_connection(&mut self, source: &str, token: &str, org: &str, base: &str) -> Result<()> {
        match source {
            "github" => self.github_token = token.into(),
            "linear" => self.linear_token = token.into(),
            "sentry" => {
                self.sentry_token = token.into();
                self.sentry_org = org.into();
                self.sentry_base = base.into();
            }
            _ => return Err(Error::new("Unknown connection provider.")),
        }
        if !token.is_empty() && !self.redaction_secrets.iter().any(|secret| secret == token) {
            self.redaction_secrets.push(token.into());
            self.redaction_secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        }
        Ok(())
    }
    pub(super) fn retain_redactions(&mut self, previous: &Self) {
        for secret in &previous.redaction_secrets {
            if !self.redaction_secrets.contains(secret) {
                self.redaction_secrets.push(secret.clone());
            }
        }
        self.redaction_secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    }
    pub(super) fn stream_redactor(&self) -> StreamRedactor {
        StreamRedactor { secrets: self.redaction_secrets.clone(), pending: String::new(), suppressed: false }
    }
    pub fn redact_text(&self, value: &mut String) {
        for secret in &self.redaction_secrets {
            *value = value.replace(secret, "[redacted]");
        }
    }
    pub fn sentry_configured(&self) -> bool {
        !self.sentry_org.is_empty() && !self.sentry_token.is_empty()
    }
    fn remote_error(&self, source: &str, response: reqwest::blocking::Response) -> Error {
        let status = response.status().as_u16();
        let mut e=Error::new(format!("{source} returned HTTP {status}. Check network access and account permissions. Cached data remains available."));
        e.retry = response
            .headers()
            .get("retry-after")
            .and_then(|x| x.to_str().ok())
            .and_then(|x| x.parse::<f64>().ok())
            .unwrap_or(300.0)
            .min(86400.0);
        e.data = json!({"kind":"http","httpStatus":status,"transport":"http"});
        e
    }
    fn graphql(&self, url: &str, token: &str, query: &str, source: &str) -> Result<Value> {
        let response=self.client.post(url).header("Authorization",token).json(&json!({"query":query})).send().map_err(|e|{let mut error=Error::new(format!("{source} {}. Cached data remains available.",if e.is_timeout(){"timed out"}else{"connection failed"}));error.data=json!({"kind":if e.is_timeout(){"timeout"}else{"network"},"transport":"http","timeoutSeconds":25});error})?;
        if !response.status().is_success() {
            return Err(self.remote_error(source, response));
        }
        let body: Value = response
            .json()
            .map_err(|_| Error::new(format!("{source} returned invalid JSON.")))?;
        if body.get("errors").is_some() {
            return Err(Error::new(format!(
                "{source} query failed. Check API permissions and limits."
            )));
        }
        Ok(body["data"].clone())
    }
    fn github(&self, query: &str) -> Result<Value> {
        if !self.github_token.is_empty() {
            return self.graphql(
                "https://api.github.com/graphql",
                &format!("Bearer {}", self.github_token),
                query,
                "GitHub",
            );
        }
        let output = super::terminal::run(
            &super::terminal::executable("gh"),
            &["api".into(), "graphql".into(), "--input".into(), "-".into()],
            Some(&json!({"query":query}).to_string()),
            35,
        )?;
        let body: Value = serde_json::from_str(&output)?;
        if body.get("errors").is_some() {
            return Err(Error::new(
                "GitHub query failed. Check API permissions and limits.",
            ));
        }
        Ok(body["data"].clone())
    }
    fn linear(&self, query: &str) -> Result<Value> {
        if self.linear_token.is_empty() {
            return Err(Error::new("Configure LINEAR_API_KEY in .env."));
        }
        self.graphql(
            "https://api.linear.app/graphql",
            &self.linear_token,
            query,
            "Linear",
        )
    }
    pub fn github_pr(&self, id: &str) -> Result<(Value,String)> {
        let query="query{viewer{login} node(id:__ID__){... on PullRequest{__FIELDS__}}}".replace("__ID__",&json!(id).to_string()).replace("__FIELDS__",text(&self.fields,"pr"));
        let data=self.github(&query)?;
        if data["node"].is_null() {return Err(Error::new("This pull request is unavailable on GitHub."));}
        Ok((data["node"].clone(),text(&data["viewer"],"login").into()))
    }
    pub fn github_pr_action(&self, id: &str, action: &str, login: &str) -> Result<(Value,String)> {
        let (before,viewer)=self.github_pr(id)?;
        if action=="refresh" {return Ok((before,viewer));}
        if before["state"]!="OPEN" {return Err(Error::new("This pull request is no longer open."));}
        if before["viewerCanUpdate"]!=true {return Err(Error::new("Your GitHub account cannot update this pull request. Check repository permissions and token write access."));}
        let (mutation,input)=match action {
            "ready" if before["isDraft"]!=true=>return Ok((before,viewer)),
            "ready"=>("markPullRequestReadyForReview",json!({"pullRequestId":id})),
            "request-review"=>{
                if before["isDraft"]==true {return Err(Error::new("Mark this draft ready before requesting a review."));}
                if text(&before["author"],"login").eq_ignore_ascii_case(login) {return Err(Error::new("The PR author cannot review their own pull request."));}
                if arr(&before["reviewRequests"]["nodes"]).iter().any(|r|text(&r["requestedReviewer"],"login").eq_ignore_ascii_case(login)){return Ok((before,viewer));}
                let user=self.github(&format!("query{{user(login:{}){{id login}}}}",json!(login)))?;
                if user["user"]["id"].is_null() {return Err(Error::new("GitHub username not found."));}
                ("requestReviews",json!({"pullRequestId":id,"userIds":[user["user"]["id"]],"union":true}))
            }
            _=>return Err(Error::new("Unsupported GitHub action.")),
        };
        // Build GraphQL input with independently encoded values, never raw UI text.
        let query=github_mutation(mutation,&input,text(&self.fields,"pr"));
        let node=github_mutation_result(self.github(&query),mutation)?;
        Ok((node,viewer))
    }
    pub fn linear_url(&self, id: &str) -> Result<String> {
        let data = self.linear(&format!("query{{issue(id:{}){{url}}}}", json!(id)))?;
        let url = safe_url(text(&data["issue"], "url"))?;
        if reqwest::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .as_deref()
            != Some("linear.app")
        {
            return Err(Error::new("Linear ticket unavailable."));
        }
        Ok(url)
    }
    pub fn fetch(&self, source: &str, environment: Option<&str>) -> Result<Vec<Value>> {
        match source {
            "github" => self.fetch_github(),
            "linear" => self.fetch_linear(),
            "sentry" => self.fetch_sentry(environment),
            _ => Err(Error::new("Unknown provider.")),
        }
    }
    fn fetch_github(&self) -> Result<Vec<Value>> {
        let login = self.github("{viewer{login}}")?["viewer"]["login"]
            .as_str()
            .ok_or(Error::new("GitHub identity unavailable."))?
            .to_string();
        let mut items: BTreeMap<String, Value> = BTreeMap::new();
        for (relation, search) in [
            ("authored", format!("is:open is:pr author:{login}")),
            ("assigned", format!("is:open is:pr assignee:{login}")),
            ("reviews", format!("is:open is:pr review-requested:{login}")),
            ("issues", format!("is:open is:issue assignee:{login}")),
        ] {
            let mut cursor = Value::Null;
            for page in 0..10 {
                let query="query { search(type: ISSUE, first: 100, query: __SEARCH__, after: __CURSOR__) { pageInfo { hasNextPage endCursor } nodes { ... on PullRequest { __PR__ } ... on Issue { __GH__ } } } }".replace("__SEARCH__",&json!(search).to_string()).replace("__CURSOR__",&cursor.to_string()).replace("__PR__",text(&self.fields,"pr")).replace("__GH__",text(&self.fields,"github"));
                let data = self.github(&query).map_err(|mut e| {
                    e.data["scope"] = json!(relation);
                    e.data["page"] = json!(page + 1);
                    e
                })?;
                let data = &data["search"];
                for node in arr(&data["nodes"]) {
                    if node.is_null() {
                        continue;
                    }
                    let mut item = github_item(&node)?;
                    let key = text(&item, "id").to_string();
                    if let Some(old) = items.get_mut(&key) {
                        old["relations"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!(relation));
                    } else {
                        item["relations"] = json!([relation]);
                        items.insert(key, item);
                    }
                }
                if data["pageInfo"]["hasNextPage"] != true {
                    break;
                }
                if page == 9 {
                    return Err(Error::new("More than 1,000 results in a GitHub search."));
                }
                cursor = data["pageInfo"]["endCursor"].clone();
            }
        }
        Ok(items.into_values().collect())
    }
    fn fetch_linear(&self) -> Result<Vec<Value>> {
        let viewer = self.linear("{viewer{id}}")?["viewer"]["id"].clone();
        let mut cursor = Value::Null;
        let mut items = vec![];
        for _ in 0..20 {
            let query = linear_query(text(&self.fields, "linear"), &viewer, &cursor);
            let data = self.linear(&query)?;
            let data = &data["issues"];
            for node in arr(&data["nodes"]) {
                let mut item = linear_item(&node)?;
                let mut roles = vec![];
                if node["assignee"]["id"] == viewer {
                    roles.push("assigned");
                }
                if node["creator"]["id"] == viewer {
                    roles.push("created");
                }
                if !arr(&node["subscribers"]["nodes"]).is_empty() {
                    roles.push("subscribed");
                }
                if !arr(&node["comments"]["nodes"]).is_empty() {
                    roles.push("commented");
                }
                if !arr(&item["sentryLinks"]).is_empty() {
                    roles.push("sentry");
                }
                item["relations"] = json!(roles);
                items.push(item);
            }
            if data["pageInfo"]["hasNextPage"] != true {
                return Ok(items);
            }
            cursor = data["pageInfo"]["endCursor"].clone();
        }
        Err(Error::new("More than 2,000 related Linear issues."))
    }
    fn sentry(&self, path: &str, params: &[(String, String)]) -> Result<(Value, String)> {
        if !self.sentry_configured() {
            return Err(Error::new(
                "Configure SENTRY_ORG and a read-only Sentry token.",
            ));
        }
        let response = self
            .client
            .get(format!("{}{path}", self.sentry_base))
            .bearer_auth(&self.sentry_token)
            .query(params)
            .send()
            .map_err(|e| {
                let mut x = Error::new(if e.is_timeout() {
                    "Sentry timed out. Cached issues remain available."
                } else {
                    "Sentry connection failed. Cached issues remain available."
                });
                x.data =
                    json!({"kind":if e.is_timeout(){"timeout"}else{"network"},"timeoutSeconds":25});
                x
            })?;
        if response.status().as_u16() == 404 {
            return Ok((Value::Null, String::new()));
        }
        if !response.status().is_success() {
            return Err(self.remote_error("Sentry", response));
        }
        let links = response
            .headers()
            .get("link")
            .and_then(|x| x.to_str().ok())
            .unwrap_or("")
            .to_string();
        Ok((
            response
                .json()
                .map_err(|_| Error::new("Sentry returned invalid JSON."))?,
            links,
        ))
    }
    fn fetch_sentry(&self, environment: Option<&str>) -> Result<Vec<Value>> {
        let mut params = vec![
            ("query".into(), "is:unresolved".into()),
            ("statsPeriod".into(), self.sentry_period.clone()),
            ("limit".into(), "100".into()),
            ("sort".into(), "freq".into()),
        ];
        if let Some(e) = environment {
            params.push(("environment".into(), e.into()));
        }
        let mut result = vec![];
        let re = regex::Regex::new("cursor=\"([^\"]+)\"").unwrap();
        for page in 1..=20 {
            let (data, links) = self
                .sentry(
                    &format!("/api/0/organizations/{}/issues/", self.sentry_org),
                    &params,
                )
                .map_err(|mut e| {
                    e.data["page"] = json!(page);
                    e.data["receivedItems"] = json!(result.len());
                    e
                })?;
            if data.is_null() {
                return Err(Error::new("Sentry organization unavailable."));
            }
            for node in arr(&data) {
                result.push(sentry_item(&node, &self.sentry_org)?);
            }
            let next = links
                .split(',')
                .find(|x| x.contains("rel=\"next\"") && x.contains("results=\"true\""));
            if let Some(cursor) = next.and_then(|x| re.captures(x)).map(|x| x[1].to_string()) {
                params.retain(|x| x.0 != "cursor");
                params.push(("cursor".into(), cursor));
            } else {
                return Ok(result);
            }
        }
        Err(Error::new(
            "More than 2,000 unresolved Sentry issues. Reduce SENTRY_STATS_PERIOD.",
        ))
    }
    pub fn resolve(&self, url: &str) -> Result<Value> {
        let url = safe_url(url)?;
        let parsed = reqwest::Url::parse(&url)
            .map_err(|_| Error::new("Use a nonempty HTTPS work URL."))?;
        if parsed.scheme() != "https" || parsed.port().is_some_and(|p| p != 443) {
            return Err(Error::new("Use an HTTPS work URL."));
        }
        let parts: Vec<_> = parsed.path().trim_matches('/').split('/').collect();
        match parsed.host_str().unwrap_or("") {
            "github.com"
                if parts.len() == 4
                    && ["pull", "issues"].contains(&parts[2])
                    && parts[3].chars().all(|c| c.is_ascii_digit()) =>
            {
                let field = if parts[2] == "pull" {
                    "pullRequest"
                } else {
                    "issue"
                };
                let query = format!(
                    "query{{repository(owner:{},name:{}){{{field}(number:{}){{{}}}}}}}",
                    json!(parts[0]),
                    json!(parts[1]),
                    parts[3],
                    text(&self.fields, if field == "issue" { "github" } else { "pr" })
                );
                let data = self.github(&query)?;
                if data["repository"][field].is_null() {
                    return Err(Error::new("GitHub item unavailable."));
                }
                github_item(&data["repository"][field])
            }
            "linear.app"
                if parts.len() >= 3
                    && parts[1] == "issue"
                    && regex::Regex::new(r"^[A-Za-z][A-Za-z0-9]*-[0-9]+$")
                        .unwrap()
                        .is_match(parts[2]) =>
            {
                let data = self.linear(
                    &"query { issue(id: __ID__) { __FIELDS__ } }"
                        .replace("__ID__", &json!(parts[2]).to_string())
                        .replace("__FIELDS__", text(&self.fields, "linear")),
                )?;
                if data["issue"].is_null() {
                    return Err(Error::new("Linear issue unavailable."));
                }
                linear_item(&data["issue"])
            }
            host if host == "sentry.io" || host.ends_with(".sentry.io") => {
                if (parts.first() == Some(&"organizations")
                    && parts.get(1) != Some(&self.sentry_org.as_str()))
                    || (host != "sentry.io"
                        && !["us.sentry.io", "de.sentry.io"].contains(&host)
                        && host != format!("{}.sentry.io", self.sentry_org))
                {
                    return Err(Error::new(
                        "Use an issue in the configured Sentry organization.",
                    ));
                }
                let id = parts.last().unwrap_or(&"");
                if id.is_empty()
                    || !id.chars().all(|c| c.is_ascii_digit())
                    || !parts.contains(&"issues")
                {
                    return Err(Error::new("Use a Sentry issue URL."));
                }
                let (data, _) = self.sentry(
                    &format!("/api/0/organizations/{}/issues/{id}/", self.sentry_org),
                    &[],
                )?;
                if data.is_null() {
                    let mut error = Error::new("Sentry issue unavailable.");
                    error.status = 404;
                    return Err(error);
                }
                sentry_item(&data, &self.sentry_org)
            }
            _ => Err(Error::new("Use a GitHub, Linear, or Sentry issue URL.")),
        }
    }
    pub fn refresh(&self, source: &str, refs: &[Value]) -> Result<Vec<Value>> {
        if refs.len() > 1000 {
            return Err(Error::new("Too many linked items need refresh."));
        }
        let mut result = vec![];
        for batch in refs.chunks(100) {
            match source {
                "github" => {
                    let ids = arr(&json!(batch))
                        .iter()
                        .map(|x| text(x, "id").trim_start_matches("github:").to_string())
                        .collect::<Vec<_>>();
                    let query = format!(
                        "query{{nodes(ids:{}){{... on PullRequest{{{}}} ... on Issue{{{}}}}}}}",
                        json!(ids),
                        text(&self.fields, "pr"),
                        text(&self.fields, "github")
                    );
                    for node in arr(&self.github(&query)?["nodes"]) {
                        if !node.is_null() {
                            result.push(github_item(&node)?);
                        }
                    }
                }
                "linear" => {
                    let ids = batch
                        .iter()
                        .map(|x| text(x, "id").trim_start_matches("linear:"))
                        .collect::<Vec<_>>();
                    let data=self.linear(&format!("query{{issues(first:100,includeArchived:true,filter:{{id:{{in:{}}}}}){{nodes{{{}}}}}}}",json!(ids),text(&self.fields,"linear")))?;
                    for node in arr(&data["issues"]["nodes"]) {
                        result.push(linear_item(&node)?);
                    }
                }
                "sentry" => {
                    if refs.len() > 100 {
                        return Err(Error::new(
                            "More than 100 linked Sentry issues need refresh.",
                        ));
                    }
                    for item in batch {
                        match self.resolve(text(item, "url")) {
                            Ok(value) => result.push(value),
                            Err(error) if error.status == 404 => (),
                            Err(error) => return Err(error),
                        }
                    }
                }
                _ => return Err(Error::new("Unknown provider.")),
            }
        }
        for old in refs {
            if !result.iter().any(|x| x["id"] == old["id"]) {
                let mut value = old.clone();
                value["sourceUnavailable"] = json!(true);
                result.push(value);
            }
        }
        Ok(result)
    }
}
pub fn status(name: &str, kind: &str, labels: &[Value]) -> &'static str {
    if ["completed", "canceled", "duplicate"].contains(&kind) {
        return "done";
    }
    let value = format!(
        "{name} {}",
        labels
            .iter()
            .map(|x| text(x, "name"))
            .collect::<Vec<_>>()
            .join(" ")
    )
    .to_lowercase();
    if value.contains("blocked") || value.contains("on hold") {
        "blocked"
    } else if value.contains("review") {
        "needs-review"
    } else if value.contains("draft") {
        "draft"
    } else if kind == "started" {
        "in-progress"
    } else {
        "todo"
    }
}
pub fn ci(n: &Value) -> Value {
    let roll = &n["commits"]["nodes"][0]["commit"]["statusCheckRollup"];
    if roll.is_null() {
        return json!({"state":"none","checks":[],"total":0});
    }
    let state = match text(roll, "state") {
        "SUCCESS" => "passed",
        "FAILURE" | "ERROR" => "failed",
        "PENDING" | "EXPECTED" => "pending",
        _ => "unknown",
    };
    let checks=arr(&roll["contexts"]["nodes"]).iter().filter(|x|!x.is_null()).map(|x|{let legacy=x.get("context").is_some();let raw=if legacy{text(x,"state")}else if x["status"]=="COMPLETED"{text(x,"conclusion")}else{text(x,"status")};let value=match raw{"SUCCESS"=>"passed","NEUTRAL"|"SKIPPED"=>"skipped","FAILURE"|"ERROR"|"TIMED_OUT"|"CANCELLED"|"ACTION_REQUIRED"|"STARTUP_FAILURE"|"STALE"=>"failed","PENDING"|"EXPECTED"|"QUEUED"|"IN_PROGRESS"|"WAITING"|"REQUESTED"=>"pending",_=>"unknown"};json!({"name":x[if legacy{"context"}else{"name"}],"state":value,"result":raw,"url":safe_url(text(x,if legacy{"targetUrl"}else{"detailsUrl"})).unwrap_or_default()})}).collect::<Vec<_>>();
    json!({"state":state,"checks":checks,"total":roll["contexts"]["totalCount"]})
}
// This stage is reached only after a mutation request is attempted. A transport
// failure or missing result cannot establish that GitHub left the PR unchanged.
fn github_mutation_result(result:Result<Value>,mutation:&str)->Result<Value>{
    let data=result.map_err(|mut error|{
        error.message="GitHub did not confirm this change. Refresh the PR before retrying; check repository permissions and token write access.".into();
        error.data=json!({"githubActionStage":"dispatched"});error
    })?;
    let node=data[mutation]["pullRequest"].clone();
    if node.is_null(){let mut error=Error::new("GitHub did not return the updated PR. Refresh it before retrying.");error.data=json!({"githubActionStage":"dispatched"});return Err(error);}
    Ok(node)
}
pub fn github_item(n: &Value) -> Result<Value> {
    let pr = n.get("isDraft").is_some();
    let ci = if pr { ci(n) } else { Value::Null };
    let mut state = status("", "", &arr(&n["labels"]["nodes"]));
    let mut native = "Open";
    if ["CLOSED", "MERGED"].contains(&text(n, "state")) {
        state = "done";
        native = if n["state"] == "MERGED" {
            "Merged"
        } else {
            "Closed"
        };
    } else if pr && state != "blocked" {
        state = match (n["isDraft"] == true, text(n, "reviewDecision")) {
            (true, _) => "draft",
            (_, "APPROVED")
                if ["passed", "none"].contains(&text(&ci, "state"))
                    && n["mergeable"] == "MERGEABLE"
                    && n["mergeStateStatus"] == "CLEAN" =>
            {
                "ready-to-merge"
            }
            (_, "APPROVED" | "CHANGES_REQUESTED") => "in-progress",
            _ => "needs-review",
        };
        native = if n["isDraft"] == true {
            "Draft"
        } else {
            match text(n, "reviewDecision") {
                "APPROVED" => "Approved",
                "CHANGES_REQUESTED" => "Changes requested",
                "REVIEW_REQUIRED" => "Awaiting review",
                _ => "Open",
            }
        };
    }
    let requested=arr(&n["reviewRequests"]["nodes"]).iter().map(|r|{let user=&r["requestedReviewer"];if !text(user,"login").is_empty(){json!({"login":user["login"],"kind":"user"})}else{json!({"login":format!("{}/{}",text(&user["organization"],"login"),text(user,"slug")),"name":user["name"],"kind":"team"})}}).filter(|r|text(r,"login")!="/").collect::<Vec<_>>();
    Ok(json!({"id":format!("github:{}",text(n,"id")),"source":"github","title":n["title"],"detail":text(n,"body").chars().take(10000).collect::<String>(),"url":safe_url(text(n,"url"))?,"reference":format!("#{}",n["number"]),"project":n["repository"]["nameWithOwner"],"status":state,"sourceStatus":native,"kind":if pr{"Pull request"}else{"Issue"},"relations":["linked"],"createdAt":n["createdAt"],"updatedAt":n["updatedAt"],"ci":ci,"mergeState":n["mergeStateStatus"],"mergeable":n["mergeable"],"isDraft":n["isDraft"],"githubState":n["state"],"author":n["author"]["login"],"canUpdate":n["viewerCanUpdate"],"baseRefName":n["baseRefName"],"headRefName":n["headRefName"],"headRefOid":n["headRefOid"],"headRepository":n["headRepository"]["nameWithOwner"],"requestedReviewers":if n["reviewRequests"].is_null(){Value::Null}else{json!(requested)},"reviewerCount":n["reviewRequests"]["totalCount"],"latestReviews":arr(&n["latestReviews"]["nodes"])}))
}
fn parent(n: &Value, depth: u8) -> Value {
    if n.is_null() || depth > 6 {
        return Value::Null;
    }
    json!({"id":format!("linear:{}",text(n,"id")),"reference":n["identifier"],"title":n["title"],"url":n["url"],"parent":parent(&n["parent"],depth+1)})
}
pub fn linear_item(n: &Value) -> Result<Value> {
    let links = arr(&n["attachments"]["nodes"])
        .into_iter()
        .filter(|x| {
            x["sourceType"] == "sentry"
                && reqwest::Url::parse(text(x, "url")).ok().is_some_and(|u| {
                    u.host_str()
                        .is_some_and(|h| h == "sentry.io" || h.ends_with(".sentry.io"))
                })
        })
        .map(|x| json!({"url":x["url"],"title":x["title"]}))
        .collect::<Vec<_>>();
    Ok(
        json!({"id":format!("linear:{}",text(n,"id")),"source":"linear","title":n["title"],"detail":text(n,"description").chars().take(10000).collect::<String>(),"url":safe_url(text(n,"url"))?,"reference":n["identifier"],"project":n["project"]["name"].as_str().unwrap_or("No project"),"projectId":n["project"]["id"],"projectIsTeam":n["project"].is_null(),"teamKey":n["team"]["key"],"teamName":n["team"]["name"],"status":status(text(&n["state"],"name"),text(&n["state"],"type"),&arr(&n["labels"]["nodes"])),"sourceStatus":if n["archivedAt"].is_null(){n["state"]["name"].clone()}else{json!("Archived")},"archivedAt":n["archivedAt"],"kind":"Issue","relations":["linked"],"createdAt":n["createdAt"],"updatedAt":n["updatedAt"],"parent":parent(&n["parent"],0),"sentryLinks":links,"children":arr(&n["children"]["nodes"]).iter().map(|x|json!({"id":format!("linear:{}",text(x,"id")),"reference":x["identifier"],"title":x["title"],"url":x["url"],"archivedAt":x["archivedAt"],"status":status(text(&x["state"],"name"),text(&x["state"],"type"),&[])})).collect::<Vec<_>>(),"moreChildren":n["children"]["pageInfo"]["hasNextPage"]==true}),
    )
}
pub fn sentry_item(n: &Value, org: &str) -> Result<Value> {
    let id = n["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| n["id"].to_string());
    if !id.chars().all(|x| x.is_ascii_digit()) {
        return Err(Error::new("Invalid Sentry identity."));
    }
    Ok(
        json!({"id":format!("sentry:{org}:{id}"),"source":"sentry","title":n["title"],"reference":n["shortId"].as_str().unwrap_or(&id),"url":safe_url(text(n,"permalink"))?,"project":format!("{org}/{}",text(&n["project"],"slug")),"status":if ["resolved","ignored"].contains(&text(n,"status")){"done"}else{"todo"},"sourceStatus":match text(n,"status"){"resolved"=>"Resolved","ignored"=>"Ignored",_=>"Unresolved"},"kind":"Error","detail":text(n,"culprit").chars().take(1500).collect::<String>(),"updatedAt":n["lastSeen"],"firstSeen":n["firstSeen"],"lastSeen":n["lastSeen"],"eventCount":n["count"],"userCount":n["userCount"],"level":n["level"],"priority":n["priority"],"category":n["issueCategory"].as_str().unwrap_or("error"),"relations":["sentry"]}),
    )
}

fn github_mutation(mutation:&str,input:&Value,selection:&str)->String {
    let fields=input.as_object().unwrap().iter().map(|(k,v)|format!("{k}:{v}")).collect::<Vec<_>>().join(",");
    "mutation{__MUTATION__(input:{__INPUT__}){pullRequest{__FIELDS__}}}".replace("__MUTATION__",mutation).replace("__INPUT__",&fields).replace("__FIELDS__",selection)
}
fn linear_query(fields: &str, viewer: &Value, cursor: &Value) -> String {
    r#"query { issues(first:100, after:__CURSOR__, filter:{ or:[
        {assignee:{id:{eq:__VIEWER__}}}, {creator:{id:{eq:__VIEWER__}}},
        {subscribers:{some:{id:{eq:__VIEWER__}}}}, {comments:{some:{user:{id:{eq:__VIEWER__}}}}},
        {attachments:{some:{sourceType:{eq:"sentry"}}}}
    ], state:{type:{nin:["completed","canceled","duplicate"]}} }) {
        pageInfo {hasNextPage endCursor} nodes { __FIELDS__
          assignee{id} creator{id} subscribers(first:1,filter:{id:{eq:__VIEWER__}}){nodes{id}}
          comments(first:1,filter:{user:{id:{eq:__VIEWER__}}}){nodes{id}}
        }
    } }"#
        .replace("__CURSOR__", &cursor.to_string())
        .replace("__VIEWER__", &viewer.to_string())
        .replace("__FIELDS__", fields)
}
#[cfg(test)]
mod query_tests {
    use super::*;
    #[test]
    fn resolve_rejects_empty_and_invalid_urls_without_panicking() {
        let root = tempfile::tempdir().unwrap();
        let providers = Providers::new(root.path()).unwrap();
        for url in ["", "not a URL", "http://github.com/example/repo/pull/1"] {
            assert!(providers.resolve(url).is_err());
        }
    }

    #[test]
    fn profile_credentials_stay_private_and_preserve_explicit_child_auth() {
        let root = tempfile::tempdir().unwrap();
        let sentinel_key = format!("MYDASH_TEST_SECRET_{}", uuid().replace('-', "_"));
        let sentinel_value = "benign-profile-only-sentinel";
        std::fs::write(root.path().join(".env"), format!(
            "SENTRY_ORG=fixture-org\nSENTRY_AUTH_TOKEN=benign-sentry-fixture\nLINEAR_API_KEY=benign-linear-fixture\nGITHUB_TOKEN=benign-github-fixture\n{sentinel_key}={sentinel_value}\n"
        )).unwrap();
        let providers = Providers::new(root.path()).unwrap();
        for (key, actual, fallback) in [
            ("SENTRY_AUTH_TOKEN", &providers.sentry_token, "benign-sentry-fixture"),
            ("LINEAR_API_KEY", &providers.linear_token, "benign-linear-fixture"),
            ("GITHUB_TOKEN", &providers.github_token, "benign-github-fixture"),
        ] {
            let expected = std::env::var(key).unwrap_or_else(|_| fallback.into());
            assert!(actual == &expected, "Provider credential precedence changed.");
        }
        assert!(std::env::var_os(&sentinel_key).is_none());
        let mut diagnostic = format!("Provider output: {sentinel_value}");
        providers.redact_text(&mut diagnostic);
        assert_eq!(diagnostic, "Provider output: [redacted]");
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", &format!(
                "test -z \"${{{sentinel_key}+present}}\" && test \"$ANTHROPIC_API_KEY\" = benign-user-auth"
            )])
            .env("ANTHROPIC_API_KEY", "benign-user-auth")
            .status().unwrap();
        assert!(status.success(), "Child environment isolation failed.");
    }

    #[test]
    fn profile_redaction_does_not_share_secrets_between_profiles() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join(".env"), "PROFILE_SECRET=benign-first-profile").unwrap();
        std::fs::write(second.path().join(".env"), "PROFILE_SECRET=benign-second-profile").unwrap();
        let first = Providers::new(first.path()).unwrap();
        let second = Providers::new(second.path()).unwrap();
        let mut first_output = "benign-first-profile / benign-second-profile".to_owned();
        first.redact_text(&mut first_output);
        assert_eq!(first_output, "[redacted] / benign-second-profile");
        let mut second_output = "benign-first-profile / benign-second-profile".to_owned();
        second.redact_text(&mut second_output);
        assert_eq!(second_output, "benign-first-profile / [redacted]");
    }

    #[test]
    fn github_post_dispatch_failures_mark_unknown_confirmation_without_private_error_data(){
        let mut failure=Error::new("fixture transport failed");failure.data=json!({"private":"private-provider-marker"});failure.retry=15.0;
        for result in [Err(failure),Ok(json!({"requestReviews":{"pullRequest":null}}))]{
            let error=github_mutation_result(result,"requestReviews").unwrap_err();
            assert_eq!(error.data,json!({"githubActionStage":"dispatched"}));assert!(error.message.contains("Refresh"));assert!(!error.data.to_string().contains("private-provider"));
        }
        let node=json!({"id":"PR_test","isDraft":false});assert_eq!(github_mutation_result(Ok(json!({"requestReviews":{"pullRequest":node}})),"requestReviews").unwrap(),node);
    }
    #[test]
    fn github_action_queries_encode_inputs_and_keep_other_review_requests() {
        let fields:Value=serde_json::from_str(include_str!("../../resources/provider-fields.json")).unwrap();
        for (name,input) in [("markPullRequestReadyForReview",json!({"pullRequestId":"PR_test"})),("requestReviews",json!({"pullRequestId":"PR_test","userIds":["user\"escaped"],"union":true}))] {
            let query=github_mutation(name,&input,text(&fields,"pr"));
            graphql_parser::parse_query::<String>(&query).unwrap();
            if name=="requestReviews" {assert!(query.contains("union:true"));}
        }
    }
    #[test]
    fn pr_branch_metadata_survives_normalization() {
        let item=github_item(&json!({"id":"PR_stack","number":2,"title":"Dependent PR","url":"https://github.com/org/repo/pull/2","state":"OPEN","repository":{"nameWithOwner":"org/repo"},"baseRefName":"parent","headRefName":"child","headRefOid":"fixture-head","headRepository":{"nameWithOwner":"org/repo"}})).unwrap();
        assert_eq!(item["baseRefName"],"parent");
        assert_eq!(item["headRefName"],"child");assert_eq!(item["headRefOid"],"fixture-head");
        assert_eq!(item["headRepository"],"org/repo");
    }
    #[test]
    fn provider_dates_survive_normalization_and_are_requested() {
        let fields:Value=serde_json::from_str(include_str!("../../resources/provider-fields.json")).unwrap();
        for name in ["github", "pr", "linear"] {assert!(text(&fields,name).contains("createdAt updatedAt"));}
        let node=json!({"id":"dates","url":"https://github.com/example/project/issues/1","createdAt":"2026-10-01T12:00:00Z","updatedAt":"2026-10-27T12:00:00Z"});
        for item in [github_item(&node).unwrap(),linear_item(&node).unwrap()] {
            assert_eq!(item["createdAt"],node["createdAt"]);assert_eq!(item["updatedAt"],node["updatedAt"]);
        }
    }
    #[test]
    fn related_linear_query_is_valid_graphql() {
        let fields: Value =
            serde_json::from_str(include_str!("../../resources/provider-fields.json")).unwrap();
        let query = linear_query(text(&fields, "linear"), &json!("viewer-id"), &Value::Null);
        graphql_parser::parse_query::<String>(&query).unwrap();
        assert!(query.contains("creator"));
        assert!(query.contains("subscribers"));
        assert!(query.contains("comments"));
        assert!(!query.contains("project:"));
    }
}
