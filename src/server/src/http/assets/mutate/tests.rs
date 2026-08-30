use super::*;

#[test]
fn attachment_disposition_strips_header_breaking_characters() {
    assert_eq!(
        attachment_disposition("ma\"p\\.png\r\nX-Injected: 1"),
        "attachment; filename=\"map.pngX-Injected: 1\""
    );
    assert_eq!(
        attachment_disposition("plain.png"),
        "attachment; filename=\"plain.png\""
    );
}
