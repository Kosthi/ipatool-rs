use crate::error::ClientError;

pub fn normalize_plist_xml(body: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(body);
    let text = text.trim();

    if let Some(start) = text.find("<plist")
        && let Some(end) = text.rfind("</plist>")
    {
        let plist_content = &text[start..end + "</plist>".len()];
        return plist_content.as_bytes().to_vec();
    }

    if text.starts_with("<?xml") || text.starts_with("<plist") {
        return body.to_vec();
    }

    if text.contains("<dict>") {
        let dict_start = text.find("<dict>").unwrap();
        let dict_end = text.rfind("</dict>").unwrap() + "</dict>".len();
        let dict_content = &text[dict_start..dict_end];
        return format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             {dict_content}\n\
             </plist>"
        )
        .into_bytes();
    }

    if text.contains("<key>") {
        return format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             {text}\n\
             </dict>\n\
             </plist>"
        )
        .into_bytes();
    }

    body.to_vec()
}

/// Whether `body` carries something [`parse_plist_response`] can make sense of.
///
/// Apple's edge answers a request it drops with an HTML error page or with
/// nothing at all, and those are worth telling apart from a real store reply
/// that happens to arrive under an odd status.
pub fn looks_like_plist(body: &[u8]) -> bool {
    if body.starts_with(b"bplist00") {
        return true;
    }

    // The same shapes normalize_plist_xml knows how to repair.
    let text = String::from_utf8_lossy(body);
    text.contains("<plist") || text.contains("<dict>") || text.contains("<key>")
}

pub fn parse_plist_response<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ClientError> {
    let normalized = normalize_plist_xml(body);
    let cursor = std::io::Cursor::new(&normalized);
    plist::from_reader(cursor).map_err(ClientError::PlistDe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_normalize_standard_plist() {
        let input = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>test</key>
    <string>value</string>
</dict>
</plist>"#;
        let result: HashMap<String, String> = parse_plist_response(input).unwrap();
        assert_eq!(result.get("test"), Some(&"value".to_string()));
    }

    #[test]
    fn test_normalize_wrapped_in_document() {
        let input = br#"<Document>
<plist version="1.0">
<dict>
    <key>hello</key>
    <string>world</string>
</dict>
</plist>
</Document>"#;
        let result: HashMap<String, String> = parse_plist_response(input).unwrap();
        assert_eq!(result.get("hello"), Some(&"world".to_string()));
    }

    #[test]
    fn test_normalize_bare_dict() {
        let input = br#"<dict>
    <key>foo</key>
    <string>bar</string>
</dict>"#;
        let result: HashMap<String, String> = parse_plist_response(input).unwrap();
        assert_eq!(result.get("foo"), Some(&"bar".to_string()));
    }

    #[test]
    fn test_normalize_bare_keys() {
        let input = br#"<key>name</key>
<string>test</string>"#;
        let result: HashMap<String, String> = parse_plist_response(input).unwrap();
        assert_eq!(result.get("name"), Some(&"test".to_string()));
    }

    #[test]
    fn store_replies_look_like_property_lists() {
        assert!(looks_like_plist(
            br#"<?xml version="1.0"?><plist version="1.0"><dict><key>failureType</key><string>-5000</string></dict></plist>"#
        ));
        assert!(looks_like_plist(
            b"<dict><key>foo</key><string>bar</string></dict>"
        ));
        assert!(looks_like_plist(b"<key>foo</key><string>bar</string>"));
        assert!(looks_like_plist(b"bplist00\x00\x01"));
    }

    /// What Apple's edge sends when it drops a sign-in request.
    #[test]
    fn html_error_pages_do_not() {
        assert!(!looks_like_plist(
            b"<html>\r\n<head><title>301 Moved Permanently</title></head>\r\n<body>\r\n<center><h1>301 Moved Permanently</h1></center>\r\n<hr><center>Apple</center>\r\n</body>\r\n</html>\r\n"
        ));
        assert!(!looks_like_plist(b""));
    }
}
