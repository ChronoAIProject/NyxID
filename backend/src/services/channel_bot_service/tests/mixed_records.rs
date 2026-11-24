//! Compiled unchanged into the two server test binaries. Fixtures use only local test keys.
use super::*;
use crate::services::{
    channel_adapters,
    channel_registration::{BOT_TOKEN_FIELD, RegistrationField},
    channel_verification_service::CredentialSnapshot,
    platform_entry_credentials::EntryCredentialSnapshot,
    provider_token_exchange_service::TokenExchangeCache,
};
use bson::{Binary, Document, spec::BinarySubtype};

const OWNER: &str = "record-test-owner";
const TOKEN: &str = "bot_token_encrypted";
const APP: &str = "app_secret_encrypted";
const VERIFY: &str = "lark_verification_token_encrypted";
const ENCRYPT: &str = "lark_encrypt_key_encrypted";
const PIN: &str = "registration_pin_encrypted";
const WEBHOOK: &str = "webhook_secret_encrypted";
const FIELDS: [&str; 6] = [TOKEN, APP, VERIFY, ENCRYPT, PIN, WEBHOOK];

// The same registration projection can select individual columns on either binary,
// including the PIN and webhook token used by managed setup.
const fn projected(storage: &'static str) -> RegistrationField {
    RegistrationField {
        name: storage,
        storage,
        webhook_secret: true,
        ..BOT_TOKEN_FIELD
    }
}
const COLUMNS: [RegistrationField; 6] = [
    projected(TOKEN),
    projected(APP),
    projected(VERIFY),
    projected(ENCRYPT),
    projected(PIN),
    projected(WEBHOOK),
];

async fn fingerprints(
    db: &mongodb::Database,
    adapter: &RecordingAdapter,
    bot: &ChannelBot,
) -> Document {
    let mut pins = doc! {"verification":CredentialSnapshot::local(bot).fingerprint().unwrap()};
    if bot.credential_source == "user" {
        pins.insert(
            "entry",
            EntryCredentialSnapshot::load(db, adapter, bot)
                .await
                .unwrap()
                .fingerprint()
                .unwrap(),
        );
    }
    // Aurinko persists SHA-256 of this exact column on its subscription handshake.
    if let Some(app_secret) = &bot.app_secret_encrypted {
        pins.insert(
            "aurinko_app_secret",
            hex::encode(Sha256::digest(app_secret)),
        );
    }
    pins
}

async fn create(
    db: &mongodb::Database,
    cfg: &AppConfig,
    keys: &EncryptionKeys,
    adapter: &RecordingAdapter,
) -> Document {
    let id = "manual";
    let fields = RegistrationValues(
        [
            ("app_id", "app"),
            ("app_secret", "test-secret"),
            ("verification_token", "verify"),
            ("encrypt_key", "enc"),
        ]
        .into(),
    );
    let result = persist_verified_bot(
        db,
        cfg,
        keys,
        adapter,
        OWNER,
        id,
        &fields,
        "app:test-secret",
        BotIdentity {
            platform_bot_id: id.into(),
            platform_bot_username: id.into(),
            display_name: None,
        },
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let row = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id":&result.bot.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!row.contains_key("secret_context_versions"));
    let expected = doc! {TOKEN:"app:test-secret",APP:"test-secret",VERIFY:"verify",ENCRYPT:"enc"};
    doc! {"row":row,"expected":expected}
}

async fn mutate(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    adapter: &RecordingAdapter,
    fixture: &mut Document,
    replace: bool,
) {
    let row = fixture.get_document("row").unwrap();
    let id = row.get_str("_id").unwrap();
    let bot = update_bot(
        db,
        keys,
        &reqwest::Client::new(),
        adapter,
        id,
        OWNER,
        UpdateBotParams {
            label: None,
            bot_token: None,
            x_events: None,
            app_id: None,
            app_secret: replace.then_some("new-secret"),
            verification_token: replace.then_some("new-verify"),
            encrypt_key: if replace {
                SecretPatch::Set("new-enc")
            } else {
                SecretPatch::Clear
            },
        },
    )
    .await
    .unwrap();
    let updated = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert!(!updated.contains_key("secret_context_versions"));
    if replace {
        for field in [TOKEN, APP, VERIFY, ENCRYPT] {
            assert!(row.get(field) != updated.get(field), "{field}");
        }
    } else {
        assert!(row.get(APP) == updated.get(APP));
        assert!(!updated.contains_key(ENCRYPT));
    }
    let expected = fixture.get_document_mut("expected").unwrap();
    if replace {
        expected.extend(
            doc! {TOKEN:"app:new-secret",APP:"new-secret",VERIFY:"new-verify",ENCRYPT:"new-enc"},
        );
    } else {
        expected.remove(ENCRYPT);
    }
    fixture.insert("row", updated);
    fixture.insert("pins", fingerprints(db, adapter, &bot).await);
}

async fn contextual_fixture(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    adapter: &RecordingAdapter,
    fixture: &mut Document,
) {
    let expected = fixture.get_document("expected").unwrap().clone();
    let row = fixture.get_document_mut("row").unwrap();
    let id = row.get_str("_id").unwrap().to_owned();
    let owner = row.get_str("user_id").unwrap().to_owned();
    let mut versions = Document::new();
    for (field, value) in expected {
        let path = [field.as_str()];
        let context =
            crate::crypto::record_field::RecordContext::new(COLLECTION_NAME, &id, &owner, &path)
                .unwrap();
        let bytes = keys
            .encrypt_record(value.as_str().unwrap().as_bytes(), &context)
            .await
            .unwrap();
        row.insert(
            &field,
            Binary {
                subtype: BinarySubtype::Generic,
                bytes,
            },
        );
        versions.insert(field, 3);
    }
    row.insert("secret_context_versions", versions);
    let bot = bson::from_document(row.clone()).unwrap();
    fixture.insert("pins", fingerprints(db, adapter, &bot).await);
}

async fn check(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    adapter: &RecordingAdapter,
    fixture: &Document,
    step: &str,
) {
    let expected = fixture.get_document("expected").unwrap();
    let mut row = fixture.get_document("row").unwrap().clone();
    let reject = step == "reject" || step.starts_with("swap-");
    match step {
        "swap-collection" => {
            let id = row.get_str("_id").unwrap().to_owned();
            let owner = row.get_str("user_id").unwrap().to_owned();
            for (field, value) in expected {
                let path = [field.as_str()];
                let context = crate::crypto::record_field::RecordContext::new(
                    "other_bots",
                    &id,
                    &owner,
                    &path,
                )
                .unwrap();
                row.insert(
                    field,
                    Binary {
                        subtype: BinarySubtype::Generic,
                        bytes: keys
                            .encrypt_record(value.as_str().unwrap().as_bytes(), &context)
                            .await
                            .unwrap(),
                    },
                );
            }
        }
        "swap-version" => {
            for field in expected.keys() {
                row.get_document_mut("secret_context_versions")
                    .unwrap()
                    .insert(field, 4);
            }
        }
        "swap-owner" => {
            row.insert("user_id", "other-owner");
        }
        "swap-id" => {
            row.insert("_id", "other-bot");
        }
        "swap-fields" => {
            let present: Vec<_> = FIELDS
                .iter()
                .copied()
                .filter(|f| expected.contains_key(f))
                .collect();
            for (i, f) in present.iter().enumerate() {
                row.insert(
                    *f,
                    fixture
                        .get_document("row")
                        .unwrap()
                        .get(present[(i + 1) % present.len()])
                        .unwrap()
                        .clone(),
                );
            }
        }
        _ => {}
    }
    let bot: ChannelBot = bson::from_document(row.clone()).unwrap();
    if !reject {
        assert_eq!(
            fingerprints(db, adapter, &bot).await,
            *fixture.get_document("pins").unwrap()
        );
    }
    let token = CredentialSnapshot::local(&bot).send_token(keys).await;
    if reject {
        assert!(token.is_err());
    } else {
        assert!(token.unwrap().as_str() == expected.get_str(TOKEN).unwrap());
    }
    for column in &COLUMNS {
        let projection = RegistrationDescriptor {
            fields: std::slice::from_ref(column),
            ..Default::default()
        };
        let read = projection.verification_secrets(keys, &bot).await;
        if reject && expected.contains_key(column.storage) {
            assert!(read.is_err(), "{}", column.name);
        } else {
            assert!(
                read.unwrap().get(column.name) == expected.get_str(column.storage).ok(),
                "{}",
                column.name
            );
        }
    }
    for platform in channel_adapters::registered_adapters(&Arc::new(TokenExchangeCache::new())) {
        let descriptor = platform.registration();
        let fields: Vec<_> = descriptor
            .fields
            .iter()
            .filter(|f| f.webhook_secret)
            .collect();
        let read = platform.build_verify_secrets(keys, &bot).await;
        if reject && fields.iter().any(|f| expected.contains_key(f.storage)) {
            assert!(read.is_err(), "{}", platform.platform_id());
            continue;
        }
        let secrets = read.unwrap();
        for field in fields {
            assert!(
                secrets.get(field.name) == expected.get_str(field.storage).ok(),
                "{}",
                platform.platform_id()
            );
        }
    }
}

#[tokio::test]
async fn channel_bot_mixed_records() {
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "roundtrip".into());
    let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let mut cfg = crate::test_utils::test_app_config();
    cfg.contextual_record_writes =
        step == "roundtrip" || std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true");
    cfg.allow_legacy_record_reads =
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false");
    let keys = Arc::new(EncryptionKeys::from_config(&cfg));
    let db = crate::test_utils::connect_transaction_test_database("mixed_channel_records").await;
    let adapter = RecordingAdapter {
        seen_tokens: Default::default(),
    };
    let mut bundle: Document = if matches!(
        step.as_str(),
        "roundtrip" | "write" | "contextual-fixture"
    ) {
        let manual = create(&db, &cfg, &keys, &adapter).await;
        let managed =
            crate::handlers::channel_managed_tests::mixed_managed_records(keys.clone()).await;
        let telegram =
            crate::services::telegram_new_tests::mixed_telegram_record(keys.clone()).await;
        let mut bundle = doc! {"manual":manual,"managed":&managed[0],"repair":&managed[1],"telegram-new":telegram};
        for kind in ["manual", "managed", "repair", "telegram-new"] {
            let fixture = bundle.get_document_mut(kind).unwrap();
            let row = fixture.get_document("row").unwrap();
            assert!(!row.contains_key("secret_context_versions"));
            let bot = bson::from_document(row.clone()).unwrap();
            fixture.insert("pins", fingerprints(&db, &adapter, &bot).await);
            if step == "contextual-fixture" || step == "roundtrip" {
                contextual_fixture(&db, &keys, &adapter, fixture).await;
            }
        }
        bundle
    } else {
        bson::from_slice(&std::fs::read(file.as_ref().unwrap()).unwrap()).unwrap()
    };
    match step.as_str() {
        "replace" | "clear" => {
            let fixture = bundle.get_document_mut("manual").unwrap();
            db.collection::<Document>(COLLECTION_NAME)
                .insert_one(fixture.get_document("row").unwrap())
                .await
                .unwrap();
            mutate(&db, &keys, &adapter, fixture, step == "replace").await;
        }
        "write" | "contextual-fixture" => {}
        "read" | "reject" | "roundtrip" | "swap-fields" | "swap-owner" | "swap-id"
        | "swap-collection" | "swap-version" => {
            for kind in ["manual", "managed", "repair", "telegram-new"] {
                check(
                    &db,
                    &keys,
                    &adapter,
                    bundle.get_document(kind).unwrap(),
                    &step,
                )
                .await;
            }
            return;
        }
        _ => panic!("unexpected fixture operation"),
    }
    std::fs::write(file.unwrap(), bson::to_vec(&bundle).unwrap()).unwrap();
}
