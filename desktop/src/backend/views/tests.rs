use super::*;
use crate::backend::now;
fn defaults() -> Value {
    serde_json::from_str(include_str!("../../../../static/config/views.json")).unwrap()
}
#[test]
fn defaults_and_missing_fields_have_explicit_meanings() {
    let doc = parse_document(&defaults()).unwrap();
    assert!(evaluate_at(
        &doc.views[0],
        &json!({"source":"github","kind":"Pull request","relations":["authored"],"status":"unknown"}),
        now()
    ));
    assert!(!evaluate_at(
        &doc.views[0],
        &json!({"source":"github","kind":"Pull request","relations":["authored"],"status":"done"}),
        now()
    ));
    for filter in [
        json!({"op":"notIn","field":"status","values":["done"]}),
        json!({"op":"containsNone","field":"relations","values":["reviews"]}),
    ] {
        let predicate = serde_json::from_value(filter).unwrap();
        assert!(!matches_filter_at(&predicate, &json!({}), now()));
    }
    let predicate = serde_json::from_value(json!({"op":"all","conditions":[{"op":"in","field":"source","values":["github"]},{"op":"any","conditions":[{"op":"equals","field":"pinned","value":true},{"op":"containsAny","field":"relations","values":["reviews"]}]}]})).unwrap();
    assert!(matches_filter_at(
        &predicate,
        &json!({"source":"github","relations":["reviews"]}),
        now()
    ));
    assert!(!matches_filter_at(
        &predicate,
        &json!({"source":"linear","pinned":true}),
        now()
    ));
}
#[test]
fn validation_rejects_versions_invalid_types_duplicates_and_depth() {
    let mut value = defaults();
    value["version"] = json!(2);
    assert!(parse_document(&value).is_err());
    value = defaults();
    value["views"][1]["id"] = value["views"][0]["id"].clone();
    assert!(parse_document(&value).is_err());
    value = defaults();
    value["views"][0]["filter"] = json!({"op":"containsAny","field":"status","values":["done"]});
    assert!(parse_document(&value).is_err());
    let mut deep = json!({"op":"all","conditions":[]});
    for _ in 0..5 {
        deep = json!({"op":"all","conditions":[deep]});
    }
    value = defaults();
    value["views"][0]["filter"] = deep;
    assert!(parse_document(&value).is_err());
}
#[test]
fn temporal_filters_match_utc_boundaries_and_rolling_windows() {
    let time = date_boundary("2026-10-27").unwrap();
    for source in ["linear", "github"] {
        for field in ["createdAt", "updatedAt"] {
            let within: Predicate =
                serde_json::from_value(json!({"op":"withinLast","field":field,"days":7})).unwrap();
            let older: Predicate =
                serde_json::from_value(json!({"op":"olderThan","field":field,"days":7})).unwrap();
            let mut item = json!({"source":source});
            item[field] = json!(7.0f64.mul_add(-86400.0, time));
            assert!(matches_filter_at(&within, &item, time));
            assert!(!matches_filter_at(&older, &item, time));
            item[field] = json!(7.0f64.mul_add(-86400.0, time) - 0.001);
            assert!(!matches_filter_at(&within, &item, time));
            assert!(matches_filter_at(&older, &item, time));
            item[field] = json!(time + 1.0);
            assert!(!matches_filter_at(&within, &item, time));
            let before: Predicate = serde_json::from_value(
                json!({"op":"beforeDate","field":field,"date":"2026-10-27"}),
            )
            .unwrap();
            let after: Predicate = serde_json::from_value(
                json!({"op":"onOrAfterDate","field":field,"date":"2026-10-27"}),
            )
            .unwrap();
            item[field] = json!("2026-10-27T02:00:00+02:00");
            assert!(!matches_filter_at(&before, &item, time));
            assert!(matches_filter_at(&after, &item, time));
            item[field] = json!("2026-10-26T23:59:59.999Z");
            assert!(matches_filter_at(&before, &item, time));
            assert!(!matches_filter_at(&after, &item, time));
            for bad in [
                Value::Null,
                json!(true),
                json!("yesterday"),
                json!("2026-10-27"),
                json!("2026-10-27T00:00:00"),
            ] {
                item[field] = bad;
                for filter in [&within, &older, &before, &after] {
                    assert!(!matches_filter_at(filter, &item, time));
                }
            }
        }
    }
    for filter in [
        json!({"op":"withinLast","field":"updatedAt","days":0}),
        json!({"op":"olderThan","field":"createdAt","days":3651}),
        json!({"op":"withinLast","field":"status","days":7}),
        json!({"op":"beforeDate","field":"updatedAt","date":"2026-02-30"}),
        json!({"op":"onOrAfterDate","field":"createdAt","date":"2026-2-01"}),
    ] {
        let mut doc = defaults();
        doc["views"][0]["filter"] = filter;
        assert!(parse_document(&doc).is_err());
    }
    assert!(date_boundary("2024-02-29").is_some());
}
#[test]
fn calendar_filters_use_saved_timezones_and_calendar_days_across_dst() {
    let timestamp = |value: &str| {
        exact_f64(
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .timestamp(),
        )
    };
    for (time, before, first, last, after) in [
        (
            "2026-03-30T10:00:00Z",
            "2026-03-28T22:59:59Z",
            "2026-03-28T23:00:00Z",
            "2026-03-29T21:59:59Z",
            "2026-03-29T22:00:00Z",
        ),
        (
            "2026-10-26T10:00:00Z",
            "2026-10-24T21:59:59Z",
            "2026-10-24T22:00:00Z",
            "2026-10-25T22:59:59Z",
            "2026-10-25T23:00:00Z",
        ),
    ] {
        let filter: Predicate = serde_json::from_value(json!({"op":"calendarPeriod","field":"updatedAt","period":"yesterday","timezone":"Europe/Berlin"})).unwrap();
        for (at, expected) in [(before, false), (first, true), (last, true), (after, false)] {
            assert_eq!(
                matches_filter_at(&filter, &json!({"updatedAt":at}), timestamp(time)),
                expected,
                "{at}"
            );
        }
        assert!(!matches_filter_at(&filter, &json!({}), timestamp(time)));
    }
    let time = timestamp("2027-01-01T10:00:00Z");
    for (period, at, expected) in [
        ("today", "2027-01-01T00:00:00Z", true),
        ("yesterday", "2026-12-31T23:00:00Z", true),
        ("sinceYesterday", "2026-12-31T00:00:00Z", true),
        ("sinceYesterday", "2027-01-01T09:00:00Z", true),
        ("sinceYesterday", "2026-12-30T23:59:59Z", false),
        ("thisWeek", "2026-12-28T00:00:00Z", true),
        ("thisWeek", "2026-12-27T23:59:59Z", false),
        ("lastWeek", "2026-12-21T00:00:00Z", true),
        ("lastWeek", "2026-12-28T00:00:00Z", false),
    ] {
        let filter: Predicate = serde_json::from_value(
            json!({"op":"calendarPeriod","field":"createdAt","period":period,"timezone":"UTC"}),
        )
        .unwrap();
        assert_eq!(
            matches_filter_at(&filter, &json!({"createdAt":at}), time),
            expected
        );
    }
    for filter in [
        json!({"op":"calendarPeriod","field":"updatedAt","period":"yesterday","timezone":"invalid/zone"}),
        json!({"op":"calendarPeriod","field":"updatedAt","period":"tomorrow","timezone":"UTC"}),
    ] {
        let mut doc = defaults();
        doc["views"][0]["filter"] = filter;
        assert!(parse_document(&doc).is_err());
    }
}
