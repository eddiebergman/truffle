//! Exact local manual work edges. Inferred/transitive bundles remain domain-owned.
use super::*;
use std::collections::BTreeMap;

const MAX_LINKS: usize = 2000;
type Pair = [String; 2];

#[derive(Clone, Copy)]
pub enum Change {
    Add,
    Remove,
}

fn keys(args: &Value, allowed: &[&str]) -> Result<()> {
    if args
        .as_object()
        .is_none_or(|args| args.keys().any(|key| !allowed.contains(&key.as_str())))
    {
        return Err(Error::new("Unknown manual work-link argument."));
    }
    Ok(())
}
fn identity(value: &Value) -> Result<String> {
    value
        .as_str()
        .filter(|id| {
            !id.trim().is_empty() && id.chars().count() <= 300 && !id.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .ok_or(Error::new("Supply an exact bounded work item ID."))
}
fn pair(value: &Value) -> Result<Pair> {
    let values = value
        .as_array()
        .filter(|values| values.len() == 2)
        .ok_or(Error::new("Choose two different work items."))?;
    let mut ids = [identity(&values[0])?, identity(&values[1])?];
    if ids[0] == ids[1] {
        return Err(Error::new("Choose two different work items."));
    }
    // Preserve the canonical legacy Value::to_string ordering, including escapes.
    ids.sort_by_key(|id| json!(id).to_string());
    Ok(ids)
}
fn stored(state: &Value) -> Result<Vec<Pair>> {
    let rows = state["workLinks"]
        .as_array()
        .filter(|rows| rows.len() <= MAX_LINKS)
        .ok_or(Error::new(
            "Stored manual work links exceed their supported bounds.",
        ))?;
    rows.iter().map(pair).collect()
}
fn exact_projection(repo: &Repository, value: Value) -> Result<Value> {
    let mut projected = value.clone();
    super::commands::redact(repo, &mut projected);
    if projected != value {
        return Err(Error::new(
            "Manual work-link identities cannot be safely projected.",
        ));
    }
    Ok(value)
}
fn change_in_state(
    state: &mut Value,
    catalog: &BTreeMap<String, Value>,
    ids: &Pair,
    change: Change,
) -> Result<Value> {
    if ids.iter().any(|id| !catalog.contains_key(id)) {
        return Err(Error::new("Choose two known work items."));
    }
    let links = stored(state)?;
    let existed = links.iter().any(|link| link == ids);
    let linked = matches!(change, Change::Add);
    let changed = existed != linked;
    if changed {
        let rows = state["workLinks"].as_array_mut().unwrap();
        if linked {
            if rows.len() >= MAX_LINKS {
                return Err(Error::new("Too many manual work links."));
            }
            rows.push(json!(ids));
        } else {
            rows.retain(|row| {
                !((row[0] == ids[0] && row[1] == ids[1]) || (row[0] == ids[1] && row[1] == ids[0]))
            });
        }
    }
    Ok(json!({"scope":"local","manualOnly":true,"itemIds":ids,"linked":linked,"changed":changed}))
}
pub fn change(repo: &Repository, args: &Value, change: Change) -> Result<Value> {
    keys(args, &["itemIds"])?;
    let ids = pair(&args["itemIds"])?;
    exact_projection(repo, json!(ids))?;
    repo.transaction_if_changed(|state| {
        let catalog = domain::catalog(state);
        let receipt = change_in_state(state, &catalog, &ids, change)?;
        // Reject an unsafe receipt before committing, including fixed routing fields.
        exact_projection(repo, receipt)
    })
}
pub(super) fn legacy_change(
    state: &mut Value,
    args: &Value,
    catalog: &BTreeMap<String, Value>,
    change: Change,
) -> Result<()> {
    keys(args, &["action", "itemIds"])?;
    let ids = pair(&args["itemIds"])?;
    change_in_state(state, catalog, &ids, change)?;
    Ok(())
}
pub fn read(repo: &Repository, args: &Value) -> Result<Value> {
    keys(args, &["itemId", "offset", "snapshot"])?;
    let item = args.get("itemId").map(identity).transpose()?;
    let offset = match args.get("offset") {
        Some(value) => value
            .as_u64()
            .filter(|offset| *offset <= MAX_LINKS as u64)
            .ok_or(Error::new(
                "Choose a manual work-link offset from 0 to 2000.",
            ))? as usize,
        None => 0,
    };
    let supplied = match args.get("snapshot") {
        Some(Value::String(value))
            if value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
        {
            Some(value.as_str())
        }
        None => None,
        _ => return Err(Error::new("Supply the exact manual work-link snapshot.")),
    };
    let mut links = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        stored(&state)?
    };
    links.sort_by_key(|link| json!(link).to_string());
    links.dedup();
    // Validate the complete retained set before filters/pages can hide identities.
    exact_projection(repo, json!({"itemId":item,"links":links}))?;
    let snapshot = context_kit::sha256(&serde_json::to_vec(&json!({"itemId":item,"links":links}))?);
    if offset > 0 && supplied.is_none() || supplied.is_some_and(|value| value != snapshot) {
        let mut error = Error::new(
            "Manual work links changed. Restart work.links.read without offset or snapshot.",
        );
        error.status = 409;
        return Err(error);
    }
    let links = links
        .into_iter()
        .filter(|link| item.as_ref().is_none_or(|item| link.contains(item)))
        .collect::<Vec<_>>();
    let total = links.len();
    exact_projection(
        repo,
        json!({"scope":"local","manualOnly":true,"itemId":item,"links":links.into_iter().skip(offset).take(100).map(|item_ids|json!({"itemIds":item_ids})).collect::<Vec<_>>(),
        "total":total,"offset":offset,"nextOffset":(offset + 100 < total).then_some(offset + 100),"snapshot":snapshot}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn fixture() -> (tempfile::TempDir, Arc<Repository>) {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        repo.transaction(|state| {
            state["items"] = json!([{"id":"a","source":"linear"},{"id":"b","source":"linear"},{"id":"c","source":"github"},{"id":"d","source":"sentry"}]);
            state["workLinks"] = json!([]);
            state["privateFixture"] = json!({"preserve":17});
            Ok(())
        }).unwrap();
        (temp, repo)
    }
    fn invoke(repo: &Arc<Repository>, name: &str, args: Value) -> Result<Value> {
        commands::invoke(repo, &json!({"name":name,"args":args}), true)
    }

    #[test]
    fn work_links_commands_are_exact_idempotent_and_share_the_legacy_owner() {
        let (_temp, repo) = fixture();
        let added = invoke(&repo, "work.links.add", json!({"itemIds":["b","a"]})).unwrap();
        assert_eq!(
            added,
            json!({"scope":"local","manualOnly":true,"itemIds":["a","b"],"linked":true,"changed":true})
        );
        let bytes = std::fs::read(&repo.data).unwrap();
        let revision = repo.snapshot(false)["revision"].clone();
        assert_eq!(
            invoke(&repo, "work.links.add", json!({"itemIds":["a","b"]})).unwrap()["changed"],
            false
        );
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
        assert_eq!(repo.snapshot(false)["revision"], revision);
        repo.edit(&json!({"action":"link-work","itemIds":["d","c"]}))
            .unwrap();
        assert_eq!(
            invoke(&repo, "work.links.read", json!({"itemId":"a"})).unwrap()["links"],
            json!([{"itemIds":["a","b"]}])
        );
        assert_eq!(
            invoke(&repo, "work.links.remove", json!({"itemIds":["b","a"]})).unwrap()["changed"],
            true
        );
        let before = std::fs::read(&repo.data).unwrap();
        assert_eq!(
            invoke(&repo, "work.links.remove", json!({"itemIds":["a","b"]})).unwrap()["changed"],
            false
        );
        assert_eq!(std::fs::read(&repo.data).unwrap(), before);
        assert_eq!(repo.state.lock().unwrap()["workLinks"], json!([["c", "d"]]));
        repo.edit(&json!({"action":"unlink-work","itemIds":["c","d"]}))
            .unwrap();
        assert_eq!(
            invoke(&repo, "work.links.read", json!({})).unwrap()["total"],
            0
        );
        assert_eq!(
            repo.state.lock().unwrap()["privateFixture"],
            json!({"preserve":17})
        );
        assert!(repo.running.lock().unwrap().is_empty());
        assert!(repo.cooldown.lock().unwrap().is_empty());
    }

    #[test]
    fn work_links_preserve_legacy_escaped_order_and_retained_catalog_identities() {
        let (_temp, repo) = fixture();
        let escaped = json!(["a\"", "aBé"]);
        let mut expected = arr(&escaped);
        expected.sort_by_key(Value::to_string);
        repo.transaction(|state| {
            state["items"] = json!([{"id":"a\"","source":"linear"},{"id":"aBé","source":"github"}]);
            state["notes"] = json!([{"id":"note:retained","source":"notes"}]);
            state["itemArchive"] =
                json!({"archive:retained":{"id":"archive:retained","source":"sentry"}});
            state["sessions"] = json!([{"items":[{"id":"session:retained","source":"linear"}]}]);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            invoke(&repo, "work.links.add", json!({"itemIds":escaped})).unwrap()["itemIds"],
            json!(expected)
        );
        repo.edit(&json!({"action":"link-work","itemIds":["aBé","a\""]}))
            .unwrap();
        assert_eq!(repo.state.lock().unwrap()["workLinks"], json!([expected]));
        for ids in [
            json!(["note:retained", "archive:retained"]),
            json!(["archive:retained", "session:retained"]),
        ] {
            assert_eq!(
                invoke(&repo, "work.links.add", json!({"itemIds":ids})).unwrap()["changed"],
                true
            );
        }
    }

    #[test]
    fn work_links_atomic_pair_changes_preserve_concurrent_unrelated_edges_and_inferred_relations() {
        let (_temp, repo) = fixture();
        let barrier = Arc::new(Barrier::new(3));
        let mut threads = vec![];
        for ids in [json!(["a", "b"]), json!(["c", "d"])] {
            let repo = Arc::clone(&repo);
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                invoke(&repo, "work.links.add", json!({"itemIds":ids})).unwrap()
            }));
        }
        barrier.wait();
        for thread in threads {
            assert_eq!(thread.join().unwrap()["changed"], true);
        }
        assert_eq!(
            invoke(&repo, "work.links.read", json!({})).unwrap()["links"],
            json!([{"itemIds":["a","b"]},{"itemIds":["c","d"]}])
        );
        let barrier = Arc::new(Barrier::new(3));
        let mut threads = vec![];
        for (name, ids) in [
            ("work.links.remove", json!(["a", "b"])),
            ("work.links.add", json!(["a", "c"])),
        ] {
            let repo = Arc::clone(&repo);
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                invoke(&repo, name, json!({"itemIds":ids})).unwrap()
            }));
        }
        barrier.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(
            invoke(&repo, "work.links.read", json!({})).unwrap()["links"],
            json!([{"itemIds":["a","c"]},{"itemIds":["c","d"]}])
        );
        repo.transaction(|state| {
            state["sessions"] = json!([{"items":[{"id":"a"},{"id":"c"}]}]);
            Ok(())
        })
        .unwrap();
        let receipt = invoke(&repo, "work.links.remove", json!({"itemIds":["a","c"]})).unwrap();
        assert_eq!(receipt["linked"], false);
        assert_eq!(receipt["manualOnly"], true);
        assert!(arr(&domain::bundles(&repo.state.lock().unwrap()))
            .iter()
            .any(|bundle| arr(&bundle["itemIds"]).contains(&json!("a"))
                && arr(&bundle["itemIds"]).contains(&json!("c"))));
    }

    #[test]
    fn work_links_pagination_checks_complete_set_and_filter_without_writes() {
        let (_temp, repo) = fixture();
        repo.transaction(|state| {
            state["workLinks"] = json!((0..110)
                .map(|i| json!(["a", format!("other:{i:03}")]))
                .collect::<Vec<_>>());
            Ok(())
        })
        .unwrap();
        let before = std::fs::read(&repo.data).unwrap();
        let first = invoke(&repo, "work.links.read", json!({})).unwrap();
        assert_eq!(first["total"], 110);
        assert_eq!(first["links"].as_array().unwrap().len(), 100);
        assert_eq!(first["nextOffset"], 100);
        let args = json!({"offset":100,"snapshot":first["snapshot"]});
        let next = invoke(&repo, "work.links.read", args.clone()).unwrap();
        assert_eq!(next["links"].as_array().unwrap().len(), 10);
        assert_eq!(next["links"][0]["itemIds"], json!(["a", "other:100"]));
        assert!(next["nextOffset"].is_null());
        assert_eq!(
            invoke(&repo, "work.links.read", json!({"itemId":"other:109"})).unwrap()["total"],
            1
        );
        for invalid in [
            json!({"offset":100}),
            json!({"itemId":"a","offset":100,"snapshot":first["snapshot"]}),
        ] {
            assert_eq!(
                invoke(&repo, "work.links.read", invalid)
                    .unwrap_err()
                    .status,
                409
            );
        }
        assert_eq!(std::fs::read(&repo.data).unwrap(), before);
        invoke(&repo, "work.links.add", json!({"itemIds":["c","d"]})).unwrap();
        assert_eq!(
            invoke(&repo, "work.links.read", args).unwrap_err().status,
            409
        );
    }

    #[test]
    fn work_links_capacity_allows_noops_and_removals_but_not_an_extra_pair() {
        let (_temp, repo) = fixture();
        repo.transaction(|state| {
            state["items"]
                .as_array_mut()
                .unwrap()
                .extend((0..2000).map(|i| json!({"id":format!("known:{i:04}"),"source":"linear"})));
            state["workLinks"] = json!((0..2000)
                .map(|i| json!(["a", format!("known:{i:04}")]))
                .collect::<Vec<_>>());
            Ok(())
        })
        .unwrap();
        let before = std::fs::read(&repo.data).unwrap();
        assert_eq!(
            invoke(
                &repo,
                "work.links.add",
                json!({"itemIds":["known:0000","a"]})
            )
            .unwrap()["changed"],
            false
        );
        assert!(invoke(&repo, "work.links.add", json!({"itemIds":["c","d"]})).is_err());
        assert_eq!(std::fs::read(&repo.data).unwrap(), before);
        invoke(
            &repo,
            "work.links.remove",
            json!({"itemIds":["known:0000","a"]}),
        )
        .unwrap();
        invoke(&repo, "work.links.add", json!({"itemIds":["c","d"]})).unwrap();
        assert_eq!(
            repo.state.lock().unwrap()["workLinks"]
                .as_array()
                .unwrap()
                .len(),
            2000
        );
    }

    #[test]
    fn work_links_invalid_arguments_and_malformed_storage_preserve_exact_state() {
        let (_temp, repo) = fixture();
        let before = std::fs::read(&repo.data).unwrap();
        let state = repo.state.lock().unwrap().clone();
        for name in ["work.links.add", "work.links.remove"] {
            for args in [
                json!({}),
                json!({"itemIds":["a"]}),
                json!({"itemIds":["a","b","c"]}),
                json!({"itemIds":["a","a"]}),
                json!({"itemIds":["a",4]}),
                json!({"itemIds":["a",""]}),
                json!({"itemIds":["a","bad\nID"]}),
                json!({"itemIds":["a","x".repeat(301)]}),
                json!({"itemIds":["a","missing"]}),
                json!({"itemIds":["a","b"],"links":[]}),
            ] {
                assert!(invoke(&repo, name, args).is_err());
            }
        }
        for args in [
            json!({"itemId":null}),
            json!({"itemId":""}),
            json!({"offset":-1}),
            json!({"offset":2001}),
            json!({"offset":1.5}),
            json!({"snapshot":"bad"}),
            json!({"all":true}),
        ] {
            assert!(invoke(&repo, "work.links.read", args).is_err());
        }
        assert!(repo
            .edit(&json!({"action":"link-work","itemIds":["a","b"],"unexpected":true}))
            .is_err());
        assert_eq!(*repo.state.lock().unwrap(), state);
        assert_eq!(std::fs::read(&repo.data).unwrap(), before);
        for malformed in [
            json!({}),
            json!([["a"]]),
            json!([["a", "a"]]),
            json!([["a", "b", "c"]]),
            json!([["a", 7]]),
            json!(vec![json!(["a", "b"]); 2001]),
        ] {
            repo.transaction(|state| {
                state["workLinks"] = malformed;
                Ok(())
            })
            .unwrap();
            let before = std::fs::read(&repo.data).unwrap();
            let state = repo.state.lock().unwrap().clone();
            assert!(invoke(&repo, "work.links.read", json!({})).is_err());
            assert!(invoke(&repo, "work.links.add", json!({"itemIds":["a","b"]})).is_err());
            assert!(repo
                .edit(&json!({"action":"unlink-work","itemIds":["a","b"]}))
                .is_err());
            assert_eq!(*repo.state.lock().unwrap(), state);
            assert_eq!(std::fs::read(&repo.data).unwrap(), before);
        }
    }

    #[test]
    fn work_links_projection_rejects_secrets_before_writes_and_before_read_pagination() {
        for secret in ["fixture-private-work-ID", "local"] {
            let temp = tempfile::tempdir().unwrap();
            std::fs::write(
                temp.path().join(".env"),
                format!("WORK_LINK_SECRET={secret}\n"),
            )
            .unwrap();
            let repo = Repository::open(temp.path().into(), None, true).unwrap();
            repo.transaction(|state| {
                state["items"] = json!([{"id":"a"},{"id":"b"},{"id":"fixture-private-work-ID"}]);
                state["workLinks"] = json!([]);
                Ok(())
            })
            .unwrap();
            let before = std::fs::read(&repo.data).unwrap();
            let state = repo.state.lock().unwrap().clone();
            let ids = if secret == "local" {
                json!(["a", "b"])
            } else {
                json!(["a", secret])
            };
            for name in ["work.links.add", "work.links.remove"] {
                let error = invoke(&repo, name, json!({"itemIds":ids})).unwrap_err();
                assert!(!error.message.contains(secret));
                assert_eq!(*repo.state.lock().unwrap(), state);
                assert_eq!(std::fs::read(&repo.data).unwrap(), before);
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let secret = "fixture-private-work-ID";
        std::fs::write(
            temp.path().join(".env"),
            format!("WORK_LINK_SECRET={secret}\n"),
        )
        .unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        repo.transaction(|state| {
            state["workLinks"] = json!((0..110)
                .map(|i| json!(["a", format!("other:{i:03}")]))
                .collect::<Vec<_>>());
            state["workLinks"]
                .as_array_mut()
                .unwrap()
                .push(json!(["z", secret]));
            Ok(())
        })
        .unwrap();
        let before = std::fs::read(&repo.data).unwrap();
        for args in [json!({}), json!({"itemId":"a"})] {
            assert!(invoke(&repo, "work.links.read", args).is_err());
            assert_eq!(std::fs::read(&repo.data).unwrap(), before);
        }
    }

    #[test]
    fn work_links_failed_save_rolls_back_then_the_same_pair_can_be_applied() {
        let (_temp, repo) = fixture();
        let before = std::fs::read(&repo.data).unwrap();
        let state = repo.state.lock().unwrap().clone();
        std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
        assert!(invoke(&repo, "work.links.add", json!({"itemIds":["a","b"]})).is_err());
        assert_eq!(*repo.state.lock().unwrap(), state);
        assert_eq!(std::fs::read(&repo.data).unwrap(), before);
        std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();
        assert_eq!(
            invoke(&repo, "work.links.add", json!({"itemIds":["a","b"]})).unwrap()["changed"],
            true
        );
    }
}
