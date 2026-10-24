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

/// Detect an HTML document before normalization can extract a dict from it.
pub(crate) fn is_html_response(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body);
    let mut text = text.trim_start_matches('\u{feff}').trim_start();
    loop {
        if text.starts_with("<?") {
            let Some(end) = text.find("?>") else { break };
            text = text[end + 2..].trim_start();
        } else if text.starts_with("<!--") {
            let Some(end) = text.find("-->") else { break };
            text = text[end + 3..].trim_start();
        } else {
            break;
        }
    }
    ["<html", "<!doctype html"].iter().any(|prefix| {
        text.get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
    })
}

pub fn parse_plist_response<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ClientError> {
    if is_html_response(body) {
        return Err(ClientError::UnexpectedResponse(
            "received HTML instead of a property list; Apple or a network intermediary returned an error page".into(),
        ));
    }
    let normalized = normalize_plist_xml(body);
    let cursor = std::io::Cursor::new(&normalized);
    plist::from_reader(cursor).map_err(ClientError::PlistDe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn html_is_reported_without_a_serde_error_or_response_body() {
        let body = b"<html>\n<head><title>Denied</title></head>\n<body>SECRET_TOKEN</body></html>";
        let err = parse_plist_response::<HashMap<String, plist::Value>>(body).unwrap_err();
        assert!(
            matches!(err, ClientError::UnexpectedResponse(_)),
            "expected an actionable HTML diagnostic, got {err}"
        );
        assert!(err.to_string().contains("HTML"));
        assert!(!err.to_string().contains("SECRET_TOKEN"));
    }

    #[test]
    fn detects_html_with_preamble_before_extracting_embedded_plist() {
        for prefix in [
            "",
            "\u{feff} \n",
            "<?xml version=\"1.0\"?>\n<!-- error -->\n",
        ] {
            for document in ["<HTML>", "<!DOCTYPE HTML>"] {
                let body = format!("{prefix}{document}<plist><dict/></plist></html>");
                assert!(is_html_response(body.as_bytes()));
                assert!(matches!(
                    parse_plist_response::<HashMap<String, plist::Value>>(body.as_bytes()),
                    Err(ClientError::UnexpectedResponse(_))
                ));
            }
        }
    }

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
