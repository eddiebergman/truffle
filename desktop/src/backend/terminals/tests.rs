use super::*;
use crate::backend::{json, uuid};
#[test]
fn resize_accepts_only_the_sizes_the_ui_sends() {
    let temp = tempfile::tempdir().unwrap();
    let terminals =
        Terminals::new(Host::new(temp.path(), &temp.path().join("state.json"), true).unwrap());
    let id = uuid();
    terminals.open_demo(&json!({"id":id})).unwrap();
    let generation = text(&terminals.inventory().unwrap().rows()[0], "generation").to_owned();
    let result = (|| -> Result<()> {
        for (cols, rows) in [(10, 2), (500, 200)] {
            terminals.resize(&id, &json!({"cols":cols,"rows":rows}))?;
        }
        for (size, message) in [
            (json!({"cols":9,"rows":2}), "Invalid columns."),
            (json!({"cols":501,"rows":2}), "Invalid columns."),
            (json!({"rows":2}), "Invalid columns."),
            (json!({"cols":10,"rows":1}), "Invalid rows."),
            (json!({"cols":10,"rows":201}), "Invalid rows."),
        ] {
            let error = terminals.resize(&id, &size).err();
            if error.map(|error| error.message).as_deref() != Some(message) {
                return Err(Error::new(format!(
                    "{size} was not rejected with {message}"
                )));
            }
        }
        Ok(())
    })();
    terminals.close(&id, &generation).unwrap();
    result.unwrap();
}
#[test]
fn inventory_rejects_duplicate_terminals_and_noncanonical_generations() {
    let id = uuid();
    let row = json!({"recordId":id,"generation":uuid(),"live":true});
    assert!(Inventory::new(vec![row.clone()]).is_ok());
    let mut other = row.clone();
    other["generation"] = json!(uuid());
    assert!(Inventory::new(vec![row.clone(), other]).is_err());
    for generation in [
        json!("stale-not-a-generation"),
        json!(uuid().to_uppercase()),
        json!(null),
    ] {
        let mut row = row.clone();
        row["generation"] = generation;
        assert!(Inventory::new(vec![row]).is_err());
    }
}
#[test]
fn live_prefers_the_chat_key_then_the_same_conversation() {
    let (chat, other, conversation) = (uuid(), uuid(), uuid());
    let record = json!({"id":chat,"agent":"codex","sessionId":conversation});
    let keyed =
        json!({"recordId":chat,"agent":"codex","sessionId":null,"generation":uuid(),"live":true});
    let resumed = json!({"recordId":other,"agent":"codex","sessionId":conversation,"generation":uuid(),"live":true});
    let live = |rows: Vec<Value>| Inventory::new(rows).unwrap().live(&record);
    assert_eq!(
        live(vec![resumed.clone(), keyed.clone()])
            .unwrap()
            .generation,
        text(&keyed, "generation")
    );
    let found = live(vec![resumed.clone()]).unwrap();
    assert_eq!(
        (
            found.id.as_str(),
            found.agent.as_str(),
            found.session_id.as_str()
        ),
        (other.as_str(), "codex", conversation.as_str())
    );
    for (key, value) in [
        ("live", json!(false)),
        ("agent", json!("claude")),
        ("sessionId", json!(uuid())),
    ] {
        let mut row = resumed.clone();
        row[key] = value;
        assert_eq!(live(vec![row]), None, "{key}");
    }
    let mut closed = keyed;
    closed["live"] = json!(false);
    assert_eq!(live(vec![closed]), None);
    let unregistered = json!({"id":chat,"agent":"codex","sessionId":""});
    let mut blank = resumed;
    blank["sessionId"] = json!("");
    let inventory = Inventory::new(vec![blank]).unwrap();
    assert_eq!(inventory.live(&unregistered), None);
}
#[test]
fn unclaimed_lists_live_terminals_that_no_chat_matches() {
    let (chat, orphan, conversation) = (uuid(), uuid(), uuid());
    let chats = [json!({"id":chat,"agent":"claude","sessionId":conversation})];
    let row = |id: &str, session: &str, live: bool| json!({"recordId":id,"agent":"claude","sessionId":session,"generation":uuid(),"live":live});
    let inventory = Inventory::new(vec![
        row(&chat, "", true),
        row(&uuid(), &conversation, true),
        row(&uuid(), "", false),
        row(&orphan, "", true),
    ])
    .unwrap();
    let unclaimed = inventory.unclaimed(&chats).collect::<Vec<_>>();
    assert_eq!(unclaimed.len(), 1);
    assert_eq!(unclaimed[0]["recordId"], orphan);
}
