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
