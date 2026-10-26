//! Deliberate source inventory, including test call sites. A new caller or
//! transport requires reviewing the same live consent gate before updating it.
use std::{collections::BTreeMap, path::Path};

type Inventory = BTreeMap<(String, String), usize>;

fn inventory(source: &str, file: &str, found: &mut Inventory) {
    let functions = regex::Regex::new(r"\bfn\s+(\w+)\s*\(").unwrap();
    let calls = regex::Regex::new(
        r#"\.\s*track\s*\(|TelemetryClient\s*::\s*track\s*\(|analytics_dispatch_service\s*::\s*begin\s*\(|\bCaptureJob\s*\{|"[^"\n]*/capture/[^"\n]*""#,
    ).unwrap();
    for call in calls.find_iter(source) {
        let function = functions
            .captures_iter(&source[..call.start()])
            .last()
            .map(|capture| capture[1].to_owned())
            .unwrap_or_else(|| "<module>".into());
        *found.entry((file.into(), function)).or_default() += 1;
    }
}

fn visit(root: &Path, directory: &Path, found: &mut Inventory) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            visit(root, &path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let file = path.strip_prefix(root).unwrap().to_str().unwrap();
            // This file is registered only under cfg(test); its scanner fixtures
            // deliberately contain forbidden call sites.
            if file != "telemetry/dispatch_inventory_tests.rs" {
                inventory(&std::fs::read_to_string(&path).unwrap(), file, found);
            }
        }
    }
}

#[test]
fn all_track_and_vendor_dispatch_paths_are_reviewed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Inventory::new();
    visit(&root, &root, &mut found);
    let expected: Inventory = [
        // Local test capture exercises successful provider exchange through
        // live account/device consent and verifies pre-auth dispatch denial.
        (
            "handlers/oauth.rs",
            "social_exchange_emits_canonical_telemetry_and_one_outer_audit",
            1,
        ),
        (
            "services/telemetry_erasure_service.rs",
            "held_capture_across_deletion_settles_before_vendor_delete",
            1,
        ),
        (
            "services/telemetry_erasure_service.rs",
            "weak_inherited_options_cannot_weaken_registry_dispatch_or_completion_evidence",
            1,
        ),
        ("telemetry/context.rs", "emit_event", 1),
        ("telemetry/erasure.rs", "handle", 1),
        ("telemetry/mod.rs", "<module>", 1),
        ("telemetry/mod.rs", "from_config", 1),
        ("telemetry/mod.rs", "track", 1),
        ("telemetry/mod.rs", "drain_loop", 2),
        (
            "telemetry/mod.rs",
            "from_config_prefers_configured_dsn_and_normalizes_host",
            1,
        ),
        (
            "telemetry/mod.rs",
            "track_enqueues_scrubbed_capture_job_with_common_properties",
            1,
        ),
        (
            "telemetry/mod.rs",
            "track_silently_drops_when_channel_is_full",
            2,
        ),
        (
            "telemetry/mod.rs",
            "dispatch_checks_live_revision_across_independent_clients",
            19,
        ),
    ]
    .into_iter()
    .map(|(file, function, count)| ((file.into(), function.into()), count))
    .collect();
    assert_eq!(
        found, expected,
        "Review new analytics callers/transport against the admission and final dispatch consent gates"
    );
}

#[test]
fn inventory_detects_new_qualified_multiline_and_post_test_paths() {
    let mut found = Inventory::new();
    inventory(
        r#"
        fn first() { client . track (event); }
        #[cfg(test)] mod tests { fn fixture() { TelemetryClient :: track (event); } }
        fn bypass() {
            analytics_dispatch_service :: begin (db, user, revision);
            let job = CaptureJob { };
            http.post("https://vendor.invalid/capture/");
        }
    "#,
        "new.rs",
        &mut found,
    );
    assert_eq!(found.get(&("new.rs".into(), "first".into())), Some(&1));
    assert_eq!(found.get(&("new.rs".into(), "fixture".into())), Some(&1));
    assert_eq!(found.get(&("new.rs".into(), "bypass".into())), Some(&3));
}
