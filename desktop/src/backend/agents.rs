//! The coding agents truffle launches, and every rule that differs between them.
use super::{home, json, text, Error, PathBuf, Result, Value};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "&'static str")]
pub enum Agent {
    Claude,
    Codex,
}
impl TryFrom<String> for Agent {
    type Error = String;
    fn try_from(name: String) -> std::result::Result<Self, String> {
        Self::parse(&name).map_err(|error| error.message)
    }
}
impl From<Agent> for &'static str {
    fn from(agent: Agent) -> Self {
        agent.name()
    }
}
impl Agent {
    pub const ALL: [Self; 2] = [Self::Claude, Self::Codex];
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            _ => Err(Error::new("Choose Claude or Codex.")),
        }
    }
    /// The stored form, which is also the CLI name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
    pub fn schema() -> Value {
        json!({"type":"string","enum":Self::ALL.map(Self::name)})
    }
    /// Only Codex has a desktop app that can open a chat.
    pub fn supports(self, destination: &str) -> bool {
        self == Self::Codex || destination != "desktop"
    }
    /// Claude takes a conversation ID at launch; Codex chooses its own and reports it later.
    pub const fn takes_session_id(self) -> bool {
        matches!(self, Self::Claude)
    }
    /// The review skill command configured in the settings `values`.
    pub fn review_command(self, values: &Value) -> Result<&str> {
        let key = match self {
            Self::Claude => "claudeReviewCommand",
            Self::Codex => "codexReviewCommand",
        };
        match text(values, key) {
            "" => Err(Error::new(
                "Choose a review skill in Settings → Agents & workflows.",
            )),
            command => Ok(command),
        }
    }
    /// The agent's configuration folder under `user_home`, unless its environment variable moves it.
    pub fn home_under(self, user_home: &Path) -> PathBuf {
        let (variable, folder) = match self {
            Self::Claude => ("CLAUDE_CONFIG_DIR", ".claude"),
            Self::Codex => ("CODEX_HOME", ".codex"),
        };
        std::env::var_os(variable).map_or_else(|| user_home.join(folder), PathBuf::from)
    }
    pub fn home(self) -> PathBuf {
        self.home_under(&home())
    }
    /// The saved log of conversation `id`, searched with a bounded walk.
    pub fn session_log(self, id: &str) -> Option<PathBuf> {
        let (folder, depth) = match self {
            Self::Claude => ("projects", 2),
            Self::Codex => ("sessions", 5),
        };
        find(&self.home().join(folder), id, depth, &mut 50000)
    }
    /// The variable through which a running agent exposes its own conversation ID.
    pub const fn session_env_var(self) -> &'static str {
        match self {
            Self::Claude => "CLAUDE_CODE_SESSION_ID",
            Self::Codex => "CODEX_THREAD_ID",
        }
    }
}
fn find(root: &Path, id: &str, depth: u8, budget: &mut usize) -> Option<PathBuf> {
    if depth == 0 || *budget == 0 {
        return None;
    }
    let entries = std::fs::read_dir(root).ok()?;
    for e in entries.filter_map(std::result::Result::ok) {
        *budget = budget.saturating_sub(1);
        if *budget == 0 {
            break;
        }
        let t = e.file_type().ok()?;
        let p = e.path();
        if t.is_file()
            && p.extension().is_some_and(|x| x == "jsonl")
            && e.file_name().to_string_lossy().contains(id)
        {
            return Some(p);
        }
        if t.is_dir() {
            if let Some(p) = find(&p, id, depth - 1, budget) {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;
