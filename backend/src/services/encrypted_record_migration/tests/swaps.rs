use super::*;

/// Exercise the registry dispatch used by both verification and findings, with
/// the same owner and row representation supplied by the production adapter.
pub(super) async fn assert_registry_swaps(
    entry: &Entry,
    db: &Database,
    keys: &EncryptionKeys,
    original: &Document,
    owner_field: &str,
) {
    verify(entry, db, keys, original).await.unwrap();
    for identity in ["_id", owner_field] {
        let mut changed = original.clone();
        changed.insert(identity, "different-identity");
        assert!(
            verify(entry, db, keys, &changed).await.is_err(),
            "{identity}"
        );
    }
    for &left in entry.fields {
        for &right in entry.fields {
            if left == right {
                continue;
            }
            let mut changed = original.clone();
            let (parent, name) = field_parent(&mut changed, left);
            parent.insert(name, path(original, right).unwrap().clone());
            assert!(
                verify(entry, db, keys, &changed).await.is_err(),
                "{left}/{right}"
            );
        }
    }
}

fn field_parent<'a, 'b>(row: &'a mut Document, path: &'b str) -> (&'a mut Document, &'b str) {
    let mut parent = row;
    let mut parts = path.split('.').peekable();
    while let Some(key) = parts.next() {
        if parts.peek().is_none() {
            return (parent, key);
        }
        parent = parent.get_document_mut(key).unwrap();
    }
    unreachable!()
}
