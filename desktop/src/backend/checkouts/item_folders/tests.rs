use super::*;
#[test]
fn an_implicit_folder_is_remembered_without_marking_it_explicit() {
    let mut state = json!({});
    remember(
        &mut state,
        "a",
        "/one",
        Strength::Implicit,
        Remember::Replace,
    );
    assert_eq!(state["itemSettings"]["a"], json!({"cwd":"/one"}));
    assert_eq!(remembered(&state, "a"), Some(("/one", Strength::Implicit)));
}
#[test]
fn an_explicit_folder_stays_explicit_after_an_implicit_one_replaces_it() {
    let mut state = json!({"itemSettings":{"a":{"cwd":"/old","futureNs":1}}});
    remember(
        &mut state,
        "a",
        "/one",
        Strength::Explicit,
        Remember::Replace,
    );
    assert_eq!(remembered(&state, "a"), Some(("/one", Strength::Explicit)));
    remember(
        &mut state,
        "a",
        "/two",
        Strength::Implicit,
        Remember::Replace,
    );
    assert_eq!(
        state["itemSettings"]["a"],
        json!({"cwd":"/two","explicit":true,"futureNs":1})
    );
}
#[test]
fn preserve_keeps_a_different_folder_and_fills_an_empty_one() {
    for strength in [Strength::Implicit, Strength::Explicit] {
        let mut state = json!({"itemSettings":{"kept":{"cwd":"/old"},"empty":{"cwd":""}}});
        let before = state.clone();
        remember(&mut state, "kept", "/new", strength, Remember::Preserve);
        assert_eq!(state, before);
        remember(&mut state, "empty", "/new", strength, Remember::Preserve);
        remember(&mut state, "absent", "/new", strength, Remember::Preserve);
        assert_eq!(remembered(&state, "empty"), Some(("/new", strength)));
        assert_eq!(remembered(&state, "absent"), Some(("/new", strength)));
        remember(&mut state, "kept", "/new", strength, Remember::Replace);
        assert_eq!(remembered(&state, "kept"), Some(("/new", strength)));
    }
}
#[test]
fn preserve_marks_the_same_folder_explicit() {
    let mut state = json!({"itemSettings":{"a":{"cwd":"/one"}}});
    remember(
        &mut state,
        "a",
        "/one",
        Strength::Explicit,
        Remember::Preserve,
    );
    assert_eq!(remembered(&state, "a"), Some(("/one", Strength::Explicit)));
}
#[test]
fn an_explicit_flag_without_a_folder_remembers_nothing() {
    let state = json!({"itemSettings":{"a":{"explicit":true}}});
    assert_eq!(remembered(&state, "a"), None);
    assert_eq!(remembered(&state, "missing"), None);
}
