use super::SourceVersion;

#[test]
fn source_version_preserves_the_existing_wire_shape() {
    let value = serde_json::json!({
        "incarnation": "01a17e12-b8e1-7601-9816-4f32046b2f5b",
        "sequence": 17,
    });
    let version: SourceVersion = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(version.sequence, 17);
    assert_eq!(serde_json::to_value(version).unwrap(), value);
}

#[test]
fn a_nil_or_missing_incarnation_is_not_a_source() {
    for value in [
        serde_json::json!({"incarnation": uuid::Uuid::nil(), "sequence": 0}),
        serde_json::json!({"sequence": 0}),
    ] {
        assert!(serde_json::from_value::<SourceVersion>(value).is_err());
    }
}
