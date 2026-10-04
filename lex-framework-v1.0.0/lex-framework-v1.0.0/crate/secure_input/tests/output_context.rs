use secure_input::encoding::{escape_js_str, escape_json_string};
#[test]
fn inline_script_string_cannot_close_its_html_script_element() {
    let payload = "</script><script>alert('世界')</script>&";
    let encoded = escape_js_str(payload);
    assert!(!encoded.contains('<'));
    assert!(!encoded.contains('>'));
    assert!(!encoded.contains('&'));
    assert!(encoded.contains("世界"));
    // JSON encoding has a different contract: it serializes data, not inline HTML.
    let encoded = escape_json_string(payload).unwrap();
    assert_eq!(serde_json::from_str::<String>(&encoded).unwrap(), payload);
}

#[test]
fn xml_text_preserves_user_data_and_rejects_unrepresentable_control_characters() {
    let input = "世界<&>\"'\n";
    let encoded = secure_input::encoding::escape_xml_text(&input).unwrap();
    assert!(!encoded.contains('<'));
    assert_eq!(quick_xml::escape::unescape(&encoded).unwrap(), input);
    assert!(secure_input::encoding::escape_xml_text(&"invalid\0XML").is_err());
}
