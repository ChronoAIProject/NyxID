use super::*;

#[test]
fn every_deployed_registry_entry_has_an_observer_and_all_lookups_are_declared() {
    let frozen: Vec<_> = families().map(|f| f.declaration()).collect();
    let mut watched: BTreeSet<String> = REGISTRY.iter().map(|e| e.collection.to_owned()).collect();
    assert_eq!((REGISTRY.len(), watched.len()), (51, 49));
    let mut joins = BTreeSet::new();
    let mut rescans = BTreeSet::new();
    for family in families() {
        for &(target, path) in family.parent_joins() {
            assert!(
                family.source_companions.iter().any(
                    |p| *p == path || path.strip_prefix(*p).is_some_and(|s| s.starts_with('.'))
                )
            );
            joins.insert((family.collection(), target, path));
            watched.insert(target.to_owned());
        }
        for &target in family.rescan_parents() {
            rescans.insert((family.collection(), target));
            watched.insert(target.to_owned());
        }
    }
    assert_eq!(
        joins,
        [
            ("assistant_agent_credentials", "api_keys", "api_key_id"),
            (
                "assistant_attachments",
                "assistant_attachments",
                "parent_attachment_id"
            ),
            (
                "platform_public_sessions",
                "api_keys",
                "credential_api_key_id"
            ),
            (
                "platform_public_turns",
                "platform_public_sessions",
                "public_session_id"
            ),
            ("device_codes", "api_keys", "issued_api_key_id"),
            ("device_codes", "nodes", "issued_node_id"),
            (
                "auth_device_codes",
                "api_key_credentials",
                "agent_key_grant.credential_id"
            ),
            (
                "auth_device_codes_v2",
                "api_key_credentials",
                "agent_key_grant.credential_id"
            ),
            ("nyxbot_channels", "api_keys", "agent_api_key_id"),
            ("nyxbot_event_bindings", "api_keys", "provider_api_key_id"),
            (
                "platform_native_reply_handles",
                "platform_native_entry_receipts",
                "receipt_id"
            )
        ]
        .into()
    );
    assert_eq!(
        rescans,
        [
            ("auth_device_codes", "refresh_tokens"),
            ("auth_device_codes_v2", "refresh_tokens")
        ]
        .into()
    );
    assert_eq!(coverage(&frozen, &watched), ManifestCoverage::Complete);
    let restored: Vec<Declaration> =
        serde_json::from_slice(&serde_json::to_vec(&frozen).unwrap()).unwrap();
    assert_eq!(coverage(&restored, &watched), ManifestCoverage::Complete);
    let mut drift = restored;
    drift[0].version += 1;
    assert_eq!(coverage(&drift, &watched), ManifestCoverage::Partial);
    for i in 0..frozen.len() {
        let mut incomplete = frozen.clone();
        incomplete.remove(i);
        assert_eq!(coverage(&incomplete, &watched), ManifestCoverage::Partial);
        let mut drift = frozen.clone();
        drift[i].fields.pop();
        assert_eq!(coverage(&drift, &watched), ManifestCoverage::Partial);
        let mut drift = frozen.clone();
        drift[i].source_companions.clear();
        assert_eq!(coverage(&drift, &watched), ManifestCoverage::Partial);
    }
    for name in &watched {
        let mut incomplete = watched.clone();
        incomplete.remove(name);
        assert_eq!(coverage(&frozen, &incomplete), ManifestCoverage::Partial);
    }
    let mut altered = frozen.clone();
    for d in &mut altered {
        d.parent_joins.clear();
        d.rescan_parents.clear();
    }
    assert_eq!(coverage(&altered, &watched), ManifestCoverage::Partial);
}
