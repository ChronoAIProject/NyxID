use super::*;

#[tokio::test]
async fn cloud_and_local_can_share_selectors_without_sharing_identity() {
    let material = Zeroizing::new(rand::random::<[u8; 32]>());
    let slot = |role, kind, name: Option<&str>| {
        Slot::new(
            role,
            kind,
            name.map(str::to_owned),
            Arc::new(LocalKeyProvider::new(*material, None)),
        )
    };
    let cloud = slot("primary", "gcp-kms", Some("fixture-logical-resource"));
    let fallback = slot("fallback", "local", None);
    let slots = [cloud, fallback];
    let id = uuid::Uuid::new_v4().to_string();
    let proofs = issue(&id, &slots).await.unwrap();
    assert_ne!(proofs[0].key.key_ref, proofs[1].key.key_ref);
    assert_eq!(proofs[0].key.internal_version, "unverified");
    assert_eq!(proofs[1].key.internal_version, "exact");
    let promoted = slot("primary", "local", None);
    assert_eq!(
        identify(&id, &promoted, &proofs).await.unwrap().key_ref,
        proofs[1].key.key_ref
    );
    assert!(
        identify(
            &id,
            &slot("primary", "gcp-kms", Some("different-resource")),
            &proofs
        )
        .await
        .is_err()
    );
    assert!(
        verify(&uuid::Uuid::new_v4().to_string(), &slots, &proofs)
            .await
            .is_err()
    );
    let mut corrupt = proofs;
    let last = corrupt[0].envelope.bytes.last_mut().unwrap();
    *last ^= 1;
    assert!(verify(&id, &slots, &corrupt).await.is_err());
}

#[tokio::test]
async fn rolling_refuses_a_provider_only_swap_with_identical_material_and_selectors() {
    use super::super::{Scope, encode, rotation};
    let id = uuid::Uuid::new_v4().to_string();
    let material = Zeroizing::new(rand::random::<[u8; 32]>());
    let other = Zeroizing::new(rand::random::<[u8; 32]>());
    let slots = [
        Slot::new(
            "primary",
            "local",
            None,
            Arc::new(LocalKeyProvider::new(*material, None)),
        ),
        Slot::new(
            "previous",
            "local",
            None,
            Arc::new(LocalKeyProvider::new(*other, None)),
        ),
    ];
    let proofs = issue(&id, &slots).await.unwrap();
    let scope = Scope {
        deployment_id: id,
        deployment_revision: "release".into(),
        restore_epoch: uuid::Uuid::new_v4().to_string(),
        keys: proofs.iter().map(|p| p.key.key_ref.clone()).collect(),
    };
    let head = bson::doc! {"managed_proofs":encode(&proofs).unwrap(),"rolling":true};
    rotation::admitted(&head, &scope, &slots).await.unwrap();
    let [_, previous] = slots;
    let changed = Slot::new(
        "primary",
        "gcp-kms",
        Some("other-resource".into()),
        Arc::new(LocalKeyProvider::new(*material, None)),
    );
    assert!(
        rotation::admitted(&head, &scope, &[changed, previous])
            .await
            .is_err()
    );
}
