use super::*;
fn progress(v: &Value) -> Value {
    if v["version"] == 1 {
        v.clone()
    } else {
        json!({"version":1,"revision":0,"days":[],"firstUsedAt":null,"star":"unseen","hint":"unseen"})
    }
}
pub fn view(repo: &Repository) -> Result<Value> {
    let config = super::config::read(&repo.data.parent().unwrap().join("config/feedback.json"))?;
    let url = reqwest::Url::parse(text(&config, "repository"))
        .map_err(|_| Error::new("Invalid feedback repository."))?;
    if config["version"] != 1
        || url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path().trim_matches('/').split('/').count() != 2
        || config["starAfterDays"]
            .as_u64()
            .is_none_or(|d| !(1..=365).contains(&d))
    {
        return Err(Error::new("Invalid feedback configuration."));
    }
    let state = repo.state.lock().map_err(|_| Error::io())?;
    let current = progress(&state["feedback"]);
    let eligible = !repo.demo
        && current["star"] == "unseen"
        && arr(&current["days"]).len() >= 2
        && current["firstUsedAt"].as_f64().is_some_and(|t| {
            now() - t >= config["starAfterDays"].as_u64().unwrap() as f64 * 86400.0
        });
    Ok(json!({"config":config,"progress":current,"starEligible":eligible,"demo":repo.demo}))
}
fn advance(previous: &Value, p: &Value) -> Result<Value> {
    if !previous.is_null() && previous["version"] != 1 {
        return Err(Error::new("Unsupported feedback state version."));
    }
    let mut next = progress(previous);
    if p["revision"].as_u64() != next["revision"].as_u64() {
        return Err(Error::new("Feedback preferences changed. Try again."));
    }
    match text(p, "action") {
        "visit" => {
            if next["firstUsedAt"].is_null() {
                next["firstUsedAt"] = json!(now());
            }
            let day = super::repository::iso_time()
                .chars()
                .take(10)
                .collect::<String>();
            let mut days = arr(&next["days"]);
            if !days.contains(&json!(day)) {
                days.push(json!(day));
            }
            days.truncate(2);
            next["days"] = json!(days);
        }
        "star-dismiss" => next["star"] = json!("dismissed"),
        "star-open" => next["star"] = json!("opened"),
        "hint-dismiss" => next["hint"] = json!("dismissed"),
        _ => return Err(Error::new("Unknown feedback action.")),
    }
    next["revision"] = json!(next["revision"].as_u64().unwrap_or(0) + 1);
    Ok(next)
}
pub fn update(repo: &Repository, p: &Value) -> Result<Value> {
    repo.transaction(|s| {
        s["feedback"] = advance(&s["feedback"], p)?;
        Ok(s["feedback"].clone())
    })
}
#[cfg(test)]
mod eligibility_tests {
    use super::*;
    #[test]
    fn stars_wait_for_two_days_and_stay_dismissed() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::open(tmp.path().into(), None, false).unwrap();
        assert_eq!(view(&repo).unwrap()["starEligible"], false);
        repo.transaction(|s|{s["feedback"]=json!({"version":1,"revision":0,"days":["first","second"],"firstUsedAt":now()-172801.0,"star":"unseen","hint":"unseen"});Ok(())}).unwrap();
        assert_eq!(view(&repo).unwrap()["starEligible"], true);
        update(&repo, &json!({"revision":0,"action":"star-dismiss"})).unwrap();
        assert_eq!(view(&repo).unwrap()["starEligible"], false);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visits_are_bounded_and_prompts_stay_dismissed() {
        let first = advance(&Value::Null, &json!({"action":"visit","revision":0})).unwrap();
        let again = advance(&first, &json!({"action":"visit","revision":1})).unwrap();
        assert_eq!(arr(&again["days"]).len(), 1);
        assert_eq!(again["firstUsedAt"], first["firstUsedAt"]);
        let closed = advance(&again, &json!({"action":"star-dismiss","revision":2})).unwrap();
        assert_eq!(closed["star"], "dismissed");
        assert!(advance(&closed, &json!({"action":"visit","revision":0})).is_err());
    }
}
