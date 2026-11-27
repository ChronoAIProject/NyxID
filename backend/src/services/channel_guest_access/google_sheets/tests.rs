use super::*;

const CONNECTION: &str = "org-google-connection";
const APPEND_QUERY: &str = "valueInputOption=RAW&insertDataOption=INSERT_ROWS";
const APPEND_BODY: &[u8] = br#"{"values":[["A question for the host"]]}"#;

fn operation(
    method: &Method,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> Option<SheetsOperation> {
    classify(
        "api-google-workspace",
        CONNECTION,
        method,
        &CanonicalPath::from_mcp_built(path).unwrap(),
        query,
        body,
        Some("application/json"),
    )
}

#[test]
fn rest_and_typed_mcp_classify_the_same_bounded_read() {
    let raw = CanonicalPath::from_rest_decoded("/v4/spreadsheets/event/values/Event_Config!A1:H24")
        .unwrap();
    let call = classify(
        "api-google-sheets",
        CONNECTION,
        &Method::GET,
        &raw,
        None,
        b"",
        None,
    )
    .unwrap();
    let typed = operation(
        &Method::GET,
        "/v4/spreadsheets/event/values/Event_Config%21A1%3AH24",
        None,
        b"",
    )
    .unwrap();
    assert_eq!(call.action, "read");
    assert_eq!(
        typed.resource_id,
        "org-google-connection/event/Event_Config!A1:H24"
    );
    assert_eq!(call.resource_id, typed.resource_id);
    assert!(validate_resource_id(&call.resource_id, call.action));
    assert!(!validate_resource_id(&call.resource_id, "create"));

    let other_workbook = operation(
        &Method::GET,
        "/v4/spreadsheets/other/values/Event_Config!A1:H24",
        None,
        b"",
    )
    .unwrap();
    let other_range = operation(
        &Method::GET,
        "/v4/spreadsheets/event/values/Event_Config!A1:H25",
        None,
        b"",
    )
    .unwrap();
    assert_ne!(other_workbook.resource_id, call.resource_id);
    assert_ne!(other_range.resource_id, call.resource_id);
    assert!(
        operation(
            &Method::GET,
            "/v4/spreadsheets/event/values/Event_Config!A:H",
            None,
            b""
        )
        .is_none()
    );
    assert!(
        operation(
            &Method::GET,
            "/v4/spreadsheets/event/values/Event_Config!A1:H1000",
            None,
            b""
        )
        .is_none()
    );
    assert!(
        operation(
            &Method::GET,
            "/v4/spreadsheets/event/values:batchGet",
            Some("ranges=Event_Config!A1:H24"),
            b""
        )
        .is_none()
    );
    assert!(
        operation(
            &Method::GET,
            "/v4/spreadsheets/event/values/Event_Config!A1:H24",
            Some("valueRenderOption=FORMULA"),
            b""
        )
        .is_none()
    );
}

#[test]
fn append_requires_the_whole_tab_grant_and_accepts_typed_unicode_path() {
    let request = "/v4/spreadsheets/event/values/%E7%99%BB%E8%AE%B0%21A%3AH:append";
    let append = operation(&Method::POST, request, Some(APPEND_QUERY), APPEND_BODY).unwrap();
    assert_eq!(append.action, "create");
    assert_eq!(append.resource_id, "org-google-connection/event/登记");
    assert!(validate_resource_id(&append.resource_id, "create"));
    assert!(!validate_resource_id(&append.resource_id, "read"));
    assert!(!validate_resource_id(
        "org-google-connection/event/登记!A:H",
        "create"
    ));

    let path = "/v4/spreadsheets/event/values/Questions!A:H:append";
    let append = operation(&Method::POST, path, Some(APPEND_QUERY), APPEND_BODY).unwrap();
    assert_eq!(append.resource_id, "org-google-connection/event/Questions");
    assert_ne!(
        operation(
            &Method::POST,
            "/v4/spreadsheets/event/values/QuestionsArchive!A:H:append",
            Some(APPEND_QUERY),
            APPEND_BODY,
        )
        .unwrap()
        .resource_id,
        append.resource_id,
    );
    assert!(supports_destination("https://sheets.googleapis.com/"));
    assert!(!supports_destination("https://www.googleapis.com/"));
    assert!(!supports_destination(
        "https://sheets.googleapis.com.example/"
    ));
}

#[test]
fn append_refuses_overwrites_ambiguous_input_extra_rows_and_echoes() {
    let path = "/v4/spreadsheets/event/values/Questions!A:H:append";
    for query in [
        None,
        Some("valueInputOption=RAW"),
        Some("valueInputOption=RAW&insertDataOption=OVERWRITE"),
        Some("valueInputOption=USER_ENTERED&insertDataOption=INSERT_ROWS"),
        Some("valueInputOption=RAW&insertDataOption=INSERT_ROWS&insertDataOption=OVERWRITE"),
        Some("valueInputOption=RAW&insertDataOption=INSERT_ROWS&includeValuesInResponse=true"),
    ] {
        assert!(operation(&Method::POST, path, query, APPEND_BODY).is_none());
    }
    for body in [
        br#"{"values":[["one"],["two"]]}"#.as_slice(),
        br#"{"values":[["one","two","three","four","five","six","seven","eight","nine"]]}"#,
        br#"{"values":[[null]]}"#,
        br#"[null,[["one"]]]"#,
        br#"{"range":"Another!A:H","values":[["one"]]}"#,
        br#"{"values":[["one"]],"values":[["two"]]}"#,
        br#"{"majorDimension":"COLUMNS","values":[["one"]]}"#,
    ] {
        assert!(operation(&Method::POST, path, Some(APPEND_QUERY), body).is_none());
    }
    assert!(operation(&Method::PUT, path, Some(APPEND_QUERY), APPEND_BODY).is_none());
    let no_sheet = "/v4/spreadsheets/event/values/A:H:append";
    assert!(operation(&Method::POST, no_sheet, Some(APPEND_QUERY), APPEND_BODY).is_none());
}
