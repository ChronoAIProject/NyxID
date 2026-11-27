use super::*;
use crate::crypto::{
    aes::EncryptionKeys,
    key_provider::{KeyProvider, Unwrapped, WrappedKey, derive_key_id},
    local_key_provider::LocalKeyProvider,
    record_field::{self, RecordContext, historical::HistoricalAad, oracle::OracleOuter},
};
use crate::{
    errors::AppError,
    test_utils::{test_app_config, test_encryption_keys},
};
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use std::sync::Arc;

const CLEAR: &[u8] = b"authenticated fixture payload";
const CURRENT: [u8; 32] = [0x11; 32];
const PREVIOUS: [u8; 32] = [0x22; 32];
fn context() -> RecordContext<'static> {
    RecordContext::new("fixtures", "row", "owner", &["secret"]).unwrap()
}
fn direct(key: &[u8; 32], nonce: [u8; 12], aad: &[u8]) -> Vec<u8> {
    let body = Aes256Gcm::new_from_slice(key)
        .unwrap()
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: CLEAR, aad })
        .unwrap();
    [nonce.as_slice(), &body].concat()
}
fn ring() -> EncryptionKeys {
    let mut config = test_app_config();
    config.encryption_key = Some(hex::encode(CURRENT));
    config.encryption_key_previous = Some(hex::encode(PREVIOUS));
    EncryptionKeys::from_config(&config)
}
fn check(open: Authenticated, format: Format, selector: Option<u8>, route: Route) {
    assert_eq!(
        format!("{open:?}"),
        format!("Authenticated {{ format: {format:?}, .. }}")
    );
    let (plaintext, evidence) = open.into_parts();
    assert_eq!(&*plaintext, CLEAR);
    assert_eq!(evidence.format, format);
    assert_eq!(evidence.selector, selector);
    assert_eq!(evidence.route, route);
    assert_eq!(format!("{:?}", evidence.route), "Route([REDACTED])");
}

#[tokio::test]
async fn direct_trials_preserve_nonce_collisions_draft_selectors_and_traffic_counts() {
    let keys = ring();
    let context = context();
    for (key, slot) in [(&CURRENT, Slot::Current), (&PREVIOUS, Slot::Previous)] {
        for prefix in [[1, 0, 0, 0], [2, 44, 0, 1], [2, 44, 4, 1], *b"\x03NYX"] {
            let mut nonce = [0x73; 12];
            nonce[..4].copy_from_slice(&prefix);
            let blob = direct(key, nonce, &[]);
            let stats = keys.decrypt_stats();
            check(
                observe(&keys, &blob, Binding::Record(None, &context))
                    .await
                    .unwrap(),
                Format::V0,
                None,
                Route::Direct(slot),
            );
            assert_eq!(keys.decrypt_stats(), stats);
            assert_eq!(&*keys.decrypt(&blob).await.unwrap(), CLEAR);
        }
        for (id, format) in [
            (derive_key_id(key), Format::V1),
            (0, Format::V1Draft),
            (1, Format::V1Draft),
        ] {
            assert!(![0, 1].contains(&derive_key_id(key)));
            let blob = [[1, id].as_slice(), &direct(key, [4; 12], &[])].concat();
            check(
                observe(&keys, &blob, Binding::Record(None, &context))
                    .await
                    .unwrap(),
                format,
                Some(id),
                Route::Direct(slot),
            );
            assert_eq!(&*keys.decrypt(&blob).await.unwrap(), CLEAR);
        }
    }
    let stats = keys.decrypt_stats();
    assert_eq!(
        (
            stats.v0_current,
            stats.v0_previous,
            stats.v1_current,
            stats.v1_previous
        ),
        (4, 4, 3, 3)
    );
    assert_eq!(
        (stats.unknown_key_id_failures, stats.decrypt_failures),
        (0, 0)
    );
}

#[tokio::test]
async fn envelope_authenticates_before_attribution_and_preserves_error_counters() {
    let keys = test_encryption_keys();
    let context = context();
    let old = ring();
    let previous_writer =
        EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new(PREVIOUS, None)));
    for version in [None, Some(3)] {
        let format = if version.is_some() {
            Format::V3
        } else {
            Format::V2
        };
        for (writer, key, slot) in [
            (&old, &CURRENT, Slot::Current),
            (&previous_writer, &PREVIOUS, Slot::Previous),
        ] {
            let blob = if version.is_some() {
                writer.encrypt_record(CLEAR, &context).await
            } else {
                writer.encrypt(CLEAR).await
            }
            .unwrap();
            check(
                observe(&old, &blob, Binding::Record(version, &context))
                    .await
                    .unwrap(),
                format,
                Some(derive_key_id(key)),
                Route::Primary(slot, LogicalKey::Local),
            );
        }
    }
    let v2 = keys.encrypt(CLEAR).await.unwrap();
    let v3 = keys.encrypt_record(CLEAR, &context).await.unwrap();
    let bad_context = RecordContext::new("fixtures", "other", "owner", &["secret"]).unwrap();
    let stats = keys.decrypt_stats();
    assert!(
        observe(&keys, &v3, Binding::Record(Some(3), &bad_context))
            .await
            .is_err()
    );
    assert!(
        observe(&keys, &v3, Binding::Record(Some(4), &context))
            .await
            .is_err()
    );
    let v0_magic = direct(&CURRENT, *b"\x03NYXabcdefgh", &[]);
    assert!(
        observe(&keys, &v0_magic, Binding::Record(Some(3), &context))
            .await
            .is_err()
    );
    for (bytes, version, len_offset) in [(&v2, None, 2), (&v3, Some(3), 5)] {
        let mut damaged = bytes.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert!(
            observe(&keys, &damaged, Binding::Record(version, &context))
                .await
                .is_err()
        );
        for len in [0u16, 1025, u16::MAX] {
            let mut broken = bytes.clone();
            broken[len_offset..len_offset + 2].copy_from_slice(&len.to_be_bytes());
            assert!(
                observe(&keys, &broken, Binding::Record(version, &context))
                    .await
                    .is_err()
            );
        }
        assert!(
            observe(
                &keys,
                &bytes[..len_offset + 1],
                Binding::Record(version, &context)
            )
            .await
            .is_err()
        );
        assert!(
            observe(
                &keys,
                &bytes[..bytes.len() - 17],
                Binding::Record(version, &context)
            )
            .await
            .is_err()
        );
    }
    assert_eq!(keys.decrypt_stats(), stats);
    let mut bad_tag = v2.clone();
    *bad_tag.last_mut().unwrap() ^= 1;
    assert!(keys.decrypt(&bad_tag).await.is_err());
    let mut expected = stats;
    expected.v2_current += 1;
    assert_eq!(keys.decrypt_stats(), expected);
}

#[tokio::test]
async fn observation_honors_typed_aad_and_sees_historical_rows_under_strict_request_policy() {
    let mut keys = ring();
    keys.set_allow_legacy_record_reads(false);
    let context = context();
    let legacy = keys.encrypt(CLEAR).await.unwrap();
    assert!(
        record_field::decrypt_context(&keys, &context, None, &legacy)
            .await
            .is_err()
    );
    check(
        observe(&keys, &legacy, Binding::Record(None, &context))
            .await
            .unwrap(),
        Format::V2,
        Some(derive_key_id(&CURRENT)),
        Route::Primary(Slot::Current, LogicalKey::Local),
    );
    let historical = HistoricalAad::mobile_delivery("hash", "challenge", "user");
    let wrong = HistoricalAad::mobile_delivery("other", "challenge", "user");
    for (id, fmt) in [
        (None, Format::V0),
        (Some(0), Format::V1Draft),
        (Some(derive_key_id(&PREVIOUS)), Format::V1),
    ] {
        let mut blob = id.map(|id| vec![1, id]).unwrap_or_default();
        blob.extend(direct(&PREVIOUS, [5; 12], historical.as_bytes()));
        check(
            observe(&keys, &blob, Binding::Historical(&historical))
                .await
                .unwrap(),
            fmt,
            id,
            Route::Direct(Slot::Previous),
        );
        assert!(
            observe(&keys, &blob, Binding::Historical(&wrong))
                .await
                .is_err()
        );
        assert_eq!(&*historical.open(&keys, &blob).await.unwrap(), CLEAR);
    }
    let bound = historical.seal(&keys, CLEAR).await.unwrap();
    assert!(
        observe(&keys, &bound, Binding::Historical(&wrong))
            .await
            .is_err()
    );
    check(
        observe(&keys, &bound, Binding::Historical(&historical))
            .await
            .unwrap(),
        Format::V2,
        Some(derive_key_id(&CURRENT)),
        Route::Primary(Slot::Current, LogicalKey::Local),
    );
    let outer = OracleOuter::snapshot("pool", "snapshot", 1);
    let other = OracleOuter::snapshot("pool", "other", 1);
    let blob = outer.seal(&keys, CLEAR).await.unwrap();
    assert!(
        observe(&keys, &blob, Binding::Oracle(&other))
            .await
            .is_err()
    );
    check(
        observe(&keys, &blob, Binding::Oracle(&outer))
            .await
            .unwrap(),
        Format::V2,
        Some(derive_key_id(&CURRENT)),
        Route::Primary(Slot::Current, LogicalKey::Local),
    );
    let direct_outer = direct(&CURRENT, [8; 12], outer.0.as_bytes());
    assert!(
        observe(&keys, &direct_outer, Binding::Oracle(&outer))
            .await
            .is_err()
    );
}

#[derive(Clone, Copy)]
enum Kms {
    Aws(Option<&'static str>),
    Gcp(&'static str),
}

struct MockKms {
    resource: Kms,
    key: LocalKeyProvider,
    unavailable: bool,
}

impl fmt::Debug for MockKms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MockKms([REDACTED])")
    }
}

#[async_trait::async_trait]
impl KeyProvider for MockKms {
    async fn wrap_dek(&self, dek: &[u8]) -> Result<WrappedKey, AppError> {
        self.key.wrap_dek(dek).await
    }
    async fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if self.unavailable {
            return Err(AppError::Internal("provider unavailable".into()));
        }
        self.key.unwrap_dek(wrapped).await
    }
    async fn unwrap_for_observation(&self, wrapped: &WrappedKey) -> Result<Unwrapped, AppError> {
        let dek = self.unwrap_dek(wrapped).await?;
        let slot = if wrapped.key_id == self.current_key_id() {
            Slot::Current
        } else {
            Slot::Previous
        };
        Ok(match self.resource {
            Kms::Aws(arn) => Unwrapped::aws(dek, slot, arn.map(str::to_owned)),
            Kms::Gcp(name) => Unwrapped::gcp(dek, slot, name),
        })
    }
    fn current_key_id(&self) -> u8 {
        self.key.current_key_id()
    }
    fn has_key_id(&self, id: u8) -> bool {
        self.key.has_key_id(id)
    }
    fn has_previous_key(&self) -> bool {
        self.key.has_previous_key()
    }
}

#[tokio::test]
async fn colliding_provider_slots_and_cloud_identity_come_from_the_successful_unwrap() {
    const AWS: &str = "arn:aws:kms:us-east-1:123456789012:key/test-key";
    const GCP: &str = "projects/sample/locations/us/keyRings/test/cryptoKeys/example";
    let context = context();
    let writer = EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new(PREVIOUS, None)));
    let v2 = writer.encrypt(CLEAR).await.unwrap();
    let v3 = writer.encrypt_record(CLEAR, &context).await.unwrap();
    for (blob, version, format) in [(&v2, None, Format::V2), (&v3, Some(3), Format::V3)] {
        for (resource, expected) in [
            (Kms::Aws(Some(AWS)), Some(LogicalKey::Aws(AWS.into()))),
            (Kms::Gcp(GCP), Some(LogicalKey::Gcp(GCP.into()))),
            (Kms::Aws(None), None),
            (
                Kms::Aws(Some(
                    "arn:aws:kms:us-east-1:123456789012:alias/retargetable",
                )),
                None,
            ),
            (Kms::Gcp("unknown-internal-version"), None),
        ] {
            let cloud = Arc::new(MockKms {
                resource,
                key: LocalKeyProvider::new(CURRENT, Some(PREVIOUS)),
                unavailable: false,
            });
            let keys = EncryptionKeys::with_provider(cloud);
            let route = expected.map_or(Route::Unknown, |logical| {
                Route::Primary(Slot::Previous, logical)
            });
            check(
                observe(&keys, blob, Binding::Record(version, &context))
                    .await
                    .unwrap(),
                format,
                Some(derive_key_id(&PREVIOUS)),
                route,
            );
        }
        // Both providers advertise the *same* selector. A provider error only
        // permits an attempt on the fallback. Attribution requires its tag.
        let offline = Arc::new(MockKms {
            resource: Kms::Aws(Some(AWS)),
            key: LocalKeyProvider::new(PREVIOUS, None),
            unavailable: true,
        });
        let keys = EncryptionKeys::with_provider(offline.clone());
        assert!(
            observe(&keys, blob, Binding::Record(version, &context))
                .await
                .is_err()
        );
        let fallback = Arc::new(MockKms {
            resource: Kms::Gcp(GCP),
            key: LocalKeyProvider::new(CURRENT, Some(PREVIOUS)),
            unavailable: false,
        });
        let keys = EncryptionKeys::with_provider_and_fallback(offline, Some(fallback));
        let stats = keys.decrypt_stats();
        check(
            observe(&keys, blob, Binding::Record(version, &context))
                .await
                .unwrap(),
            format,
            Some(derive_key_id(&PREVIOUS)),
            Route::Fallback(Slot::Previous, LogicalKey::Gcp(GCP.into())),
        );
        let mut bad = blob.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(
            observe(&keys, &bad, Binding::Record(version, &context))
                .await
                .is_err()
        );
        assert_eq!(keys.decrypt_stats(), stats);
        assert_eq!(
            &*record_field::decrypt_context(&keys, &context, version.map(|n| n as u8), blob)
                .await
                .unwrap(),
            CLEAR
        );
        assert_eq!(
            keys.decrypt_stats().v2_fallback,
            u64::from(version.is_none())
        );
    }
}

#[tokio::test]
async fn fallback_current_is_proved_by_unwrap_when_two_live_local_providers_share_a_selector() {
    let mut collision = CURRENT;
    let id = derive_key_id(&CURRENT);
    let alternate = (1u32..100_000)
        .find_map(|n| {
            collision[..4].copy_from_slice(&n.to_be_bytes());
            (derive_key_id(&collision) == id).then_some(collision)
        })
        .unwrap();
    assert_ne!(alternate, CURRENT);
    let writer = EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new(alternate, None)));
    let reader = EncryptionKeys::with_provider_and_fallback(
        Arc::new(LocalKeyProvider::new(CURRENT, None)),
        Some(Arc::new(LocalKeyProvider::new(alternate, None))),
    );
    let context = context();
    for (marker, format, blob) in [
        (None, Format::V2, writer.encrypt(CLEAR).await.unwrap()),
        (
            Some(3),
            Format::V3,
            writer.encrypt_record(CLEAR, &context).await.unwrap(),
        ),
    ] {
        check(
            observe(&reader, &blob, Binding::Record(marker, &context))
                .await
                .unwrap(),
            format,
            Some(id),
            Route::Fallback(Slot::Current, LogicalKey::Local),
        );
    }
}
