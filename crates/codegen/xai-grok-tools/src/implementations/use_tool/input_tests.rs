use super::*;
use serde_json::json;

/// Anthropic rejects tool input schemas whose top level uses `oneOf`/`anyOf`/`allOf`
/// (400: "input_schema does not support oneOf, allOf, or anyOf at the top level").
/// The file-input variant must therefore stay a flat object that lists all four
/// properties; the exclusive call-form rule is enforced at runtime, not on the wire.
#[test]
fn file_input_schema_has_no_top_level_composition_and_lists_all_properties() {
    for supports_file_input in [false, true] {
        let schema = serde_json::to_value(UseToolInput::input_schema(supports_file_input)).unwrap();
        for keyword in ["oneOf", "anyOf", "allOf", "not"] {
            assert!(
                schema.get(keyword).is_none(),
                "top-level {keyword} in input_schema({supports_file_input})"
            );
        }
        let properties = schema["properties"].as_object().unwrap();
        for property in ["tool_name", "tool_input"] {
            assert!(properties.contains_key(property), "missing {property}");
        }
        if supports_file_input {
            for property in ["tool_input_file", "file"] {
                assert!(properties.contains_key(property), "missing {property}");
            }
        } else {
            assert!(!properties.contains_key("tool_input_file"));
            assert!(!properties.contains_key("file"));
        }
    }
}

#[test]
fn all_known_field_presence_combinations_are_exclusive() {
    for mask in 0..16 {
        let mut object = serde_json::Map::new();
        for (index, key) in FIELDS.iter().enumerate() {
            if mask & (1 << index) != 0 {
                object.insert(
                    (*key).to_owned(),
                    match *key {
                        "tool_input" => Value::Null,
                        _ => json!("x"),
                    },
                );
            }
        }
        object.insert("unrelated".to_owned(), json!(true));
        let result = serde_json::from_value::<UseToolInput>(Value::Object(object));
        if [3, 5, 8].contains(&mask) {
            let parsed = result.unwrap();
            assert_eq!(
                parsed,
                serde_json::from_value(serde_json::to_value(&parsed).unwrap()).unwrap()
            );
        } else {
            assert!(
                result.unwrap_err().to_string().contains("exactly one"),
                "mask {mask}"
            );
        }
    }
}

#[test]
fn raw_duplicate_and_escaped_equivalent_keys_reject_before_mapping() {
    let mapping = HashMap::from([
        ("target".to_owned(), "tool_name".to_owned()),
        ("source".to_owned(), "file".to_owned()),
    ]);
    for json in [
        r#"{"file":"a","file":"b"}"#,
        r#"{"file":"a","f\u0069le":"b"}"#,
        r#"{"file":"a","source":"b"}"#,
        r#"{"target":"a","tool_name":"b","tool_input":{}}"#,
    ] {
        assert!(
            UseToolInput::from_model_json(json, &mapping)
                .unwrap_err()
                .to_string()
                .contains("duplicate or ambiguous")
        );
    }
}

#[test]
fn paths_and_inline_values_keep_legacy_semantics() {
    for value in [
        Value::Null,
        json!("{\"n\":1}"),
        json!(7),
        json!([]),
        json!({}),
    ] {
        let parsed: UseToolInput =
            serde_json::from_value(json!({"tool_name":"server__tool","tool_input":value})).unwrap();
        assert_eq!(
            UseToolInput::Inline(InlineMcpInvocation {
                tool_name: "server__tool".to_owned(),
                tool_input: value
            }),
            parsed
        );
    }
    for path in [Value::Null, json!(""), json!(7)] {
        assert!(
            serde_json::from_value::<UseToolInput>(json!({"file":path}))
                .unwrap_err()
                .to_string()
                .contains("nonempty string")
        );
    }
    let input: UseToolInput =
        serde_json::from_value(json!({"file":" ../unchanged path "})).unwrap();
    assert_eq!(Some(Path::new(" ../unchanged path ")), input.source_path());
}

#[test]
fn file_documents_are_strict_canonical_and_nonrecursive() {
    for document in [
        "{}",
        "null",
        "[]",
        "{} {}",
        r#"{"tool_name":"s__t","tool_input":null}"#,
        r#"{"tool_name":"s__t","tool_input":"{}"}"#,
        r#"{"tool_name":"s__t","tool_input":{},"file":null}"#,
        r#"{"tool_name":"s__t","tool_input":{},"tool_input_file":null}"#,
        r#"{"target":"s__t","args":{}}"#,
        r#"{"tool_name":"s__t","tool_name":"s__u","tool_input":{}}"#,
    ] {
        assert!(
            UseToolInput::from_invocation_file(document).is_err(),
            "{document}"
        );
    }
    let remote = json!({"file":"ordinary data", "tool_name":"not a target", "tool_input_file":"also data", "input": "{\"body\":\"exact\\ntext\"}"});
    assert_eq!(remote, parse_arguments_file(&remote.to_string()).unwrap());
    let input = UseToolInput::from_invocation_file(
        &json!({"tool_name":"s__t","tool_input":remote}).to_string(),
    )
    .unwrap();
    assert_eq!(remote, input.tool_input);
    for document in ["null", "[]", "\"{}\"", "{} trailing", ""] {
        assert!(parse_arguments_file(document).is_err(), "{document}");
    }
    assert_eq!(json!({}), parse_arguments_file("{}").unwrap());
}

#[test]
fn remapping_does_not_descend_into_remote_properties() {
    let mapping = HashMap::from([
        ("file".to_owned(), "source".to_owned()),
        ("tool_input".to_owned(), "args".to_owned()),
    ]);
    let nested = json!({"properties":{"tool_input":{"properties":{"file":{"type":"string"}},"required":["file"]}},"required":["tool_input"]});
    let mapped = crate::util::remap::remap_schema_properties(&nested, &mapping);
    assert_eq!(
        nested.get("properties").unwrap().get("tool_input"),
        mapped.pointer("/properties/args")
    );
}
