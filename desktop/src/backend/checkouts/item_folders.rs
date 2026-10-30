//! The folder remembered for each work item, stored at `itemSettings[id]`.
use super::super::{json, text, Value};
use super::Remember;

/// How the remembered folder was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::backend) enum Strength {
    /// The folder of a chat the item was linked to.
    Implicit,
    /// A folder chosen for the item. It wins over folder mappings.
    Explicit,
}
/// Remember `cwd` as the folder of `item`. An explicit choice stays explicit.
pub(in crate::backend) fn remember(
    state: &mut Value,
    item: &str,
    cwd: &str,
    strength: Strength,
    existing: Remember,
) {
    let current = &state["itemSettings"][item];
    if existing == Remember::Preserve && !text(current, "cwd").is_empty() && current["cwd"] != cwd {
        return;
    }
    let setting = &mut state["itemSettings"][item];
    if !setting.is_object() {
        *setting = json!({});
    }
    setting["cwd"] = json!(cwd);
    if strength == Strength::Explicit {
        setting["explicit"] = json!(true);
    }
}
/// The folder remembered for `item`, if any.
pub(in crate::backend) fn remembered<'a>(
    state: &'a Value,
    item: &str,
) -> Option<(&'a str, Strength)> {
    let setting = &state["itemSettings"][item];
    let cwd = text(setting, "cwd");
    if cwd.is_empty() {
        return None;
    }
    let strength = if setting["explicit"] == true {
        Strength::Explicit
    } else {
        Strength::Implicit
    };
    Some((cwd, strength))
}

#[cfg(test)]
mod tests;
