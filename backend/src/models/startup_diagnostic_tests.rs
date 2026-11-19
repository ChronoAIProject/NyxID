use super::*;
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use std::{collections::BTreeMap, path::Path};

fn fields(tokens: &[TokenTree], quoted: bool) -> Vec<String> {
    tokens
        .windows(2)
        .filter_map(|pair| {
            if !matches!(&pair[1], TokenTree::Punct(p) if p.as_char() == ':') {
                return None;
            }
            match &pair[0] {
                TokenTree::Ident(name) if !quoted => Some(name.to_string()),
                TokenTree::Literal(name) if quoted => {
                    serde_json::from_str::<String>(&name.to_string()).ok()
                }
                _ => None,
            }
        })
        .collect()
}

fn scan(stream: TokenStream, missing: &mut Vec<String>) -> usize {
    let tokens: Vec<_> = stream.into_iter().collect();
    let mut count = 0;
    let mut index = 0;
    while index < tokens.len() {
        if matches!(&tokens[index], TokenTree::Punct(p) if p.as_char() == '#')
            && matches!(tokens.get(index + 1), Some(TokenTree::Group(g))
                if g.stream().to_string().replace(' ', "") == "cfg(test)")
        {
            index += 2;
            while index < tokens.len() {
                let end = matches!(&tokens[index], TokenTree::Group(g) if g.delimiter() == Delimiter::Brace)
                    || matches!(&tokens[index], TokenTree::Punct(p) if p.as_char() == ';');
                index += 1;
                if end {
                    break;
                }
            }
            continue;
        }
        if let TokenTree::Group(group) = &tokens[index] {
            let inner: Vec<_> = group.stream().into_iter().collect();
            let previous = index.checked_sub(1).and_then(|i| tokens.get(i));
            let before_type = index.checked_sub(2).and_then(|i| tokens.get(i));
            let constructor = group.delimiter() == Delimiter::Brace
                && matches!(previous, Some(TokenTree::Ident(name)) if matches!(name.to_string().as_str(),
                    "StartupDiagnostic" | "BillingStartupDiagnostic" | "StartupDiagnosticItem"))
                && !matches!(before_type, Some(TokenTree::Ident(name)) if name == "struct")
                && !matches!(before_type, Some(TokenTree::Punct(p)) if p.as_char() == '>');
            let bson_fields = fields(&inner, true);
            let bson_diagnostic = ["code", "summary", "detail", "remediation"]
                .iter()
                .all(|key| bson_fields.iter().any(|field| field.as_str() == *key));
            if constructor || bson_diagnostic {
                count += 1;
                let explicit = if constructor {
                    // Shorthand fields still explicitly carry the caller's severity.
                    inner
                        .iter()
                        .any(|t| matches!(t, TokenTree::Ident(name) if name == "severity"))
                } else {
                    bson_fields.iter().any(|field| field == "severity")
                };
                if !explicit {
                    missing.push(group.stream().to_string());
                }
            }
            count += scan(group.stream(), missing);
        }
        index += 1;
    }
    count
}

fn inventory(root: &Path, path: &Path, found: &mut BTreeMap<String, usize>) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if name == "tests"
            || name == "tests.rs"
            || name.ends_with("_tests")
            || name.ends_with("_tests.rs")
        {
            continue;
        }
        if path.is_dir() {
            inventory(root, &path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            let mut missing = Vec::new();
            let count = scan(source.parse().unwrap(), &mut missing);
            let relative = path.strip_prefix(root).unwrap().to_str().unwrap();
            assert!(
                missing.is_empty(),
                "{relative}: diagnostics without explicit severity: {missing:?}"
            );
            if count > 0 {
                found.insert(relative.to_owned(), count);
            }
        }
    }
}

#[test]
fn integrity_every_diagnostic_producer_requires_explicit_severity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = BTreeMap::new();
    inventory(&root, &root, &mut found);
    let expected = [
        ("db.rs", 1),
        ("handlers/admin.rs", 3),
        ("services/billing/hold_conservation.rs", 1),
        ("services/billing/hold_conservation/backlog.rs", 1),
        ("services/billing/mod.rs", 5),
        ("services/billing/reconcile_status.rs", 1),
        ("services/billing/stuck_requests/inventory.rs", 1),
        ("services/billing/topup_expiry.rs", 1),
        ("services/billing/usage_comparison/diagnostics.rs", 1),
        ("services/org_service/diagnostics.rs", 1),
        ("services/platform_model_execution.rs", 1),
    ]
    .into_iter()
    .map(|(path, count)| (path.to_owned(), count))
    .collect();
    assert_eq!(
        found, expected,
        "Update the reviewed Integrity producer inventory when adding a producer"
    );
}

#[test]
fn integrity_severity_guard_detects_new_typed_and_bson_producers() {
    for source in [
        r#"fn new_code() { BillingStartupDiagnostic { code: "new".into(), ..old } }"#,
        r#"fn new_code() { doc! {"$set": {"code":"new", "summary":"s", "detail":"d", "remediation":"r"}}; }"#,
    ] {
        let mut missing = Vec::new();
        assert_eq!(scan(source.parse().unwrap(), &mut missing), 1);
        assert_eq!(missing.len(), 1);
    }
    let mut missing = Vec::new();
    assert_eq!(
        scan(
            r#"fn new_code() { BillingStartupDiagnostic { code: "new".into(), severity, ..old } }"#
                .parse()
                .unwrap(),
            &mut missing
        ),
        1
    );
    assert!(missing.is_empty());
}

#[test]
fn integrity_legacy_rows_default_to_action_and_severity_serializes_explicitly() {
    let at = bson::DateTime::from_millis(0);
    let legacy = bson::doc! {"_id":"legacy", "active":true, "code":"unknown_legacy_code",
    "summary":"Existing diagnostic", "detail":"Existing detail", "remediation":"Review",
    "detected_at":at, "updated_at":at};
    let diagnostic: StartupDiagnostic = bson::from_document(legacy).unwrap();
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Action);
    assert!(diagnostic.technical_details.is_none());
    for (severity, label) in [
        (DiagnosticSeverity::Info, "info"),
        (DiagnosticSeverity::Action, "action"),
    ] {
        let mut diagnostic = diagnostic.clone();
        diagnostic.severity = severity;
        let stored = bson::to_document(&diagnostic).unwrap();
        assert_eq!(stored.get_str("severity").unwrap(), label);
        assert_eq!(serde_json::to_value(severity).unwrap(), label);
    }
}
