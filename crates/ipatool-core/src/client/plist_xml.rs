use crate::error::ClientError;

pub fn normalize_plist_xml(body: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(body);
    let text = text.trim();

    if let Some(start) = text.find("<plist") {
        if let Some(end) = text.rfind("</plist>") {
            let plist_content = &text[start..end + "</plist>".len()];
            return plist_content.as_bytes().to_vec();
        }
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

pub fn parse_plist_response<T: serde::de::DeserializeOwned>(
    body: &[u8],
) -> Result<T, ClientError> {
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
}
