"""Exact Rust definition and backend source pairs reviewed for the raw AES boundary."""

AES = "nyxid_server::crypto::aes"
KEYS = AES + "::EncryptionKeys"
OBSERVE = "nyxid_server::crypto::record_field::authenticated::observe"
CENSUS = "services/encrypted_record_census"
CENSUS_OBSERVATION = "services/encrypted_record_migration/observation"
OBSERVATION_UNWRAP = "nyxid_server::crypto::key_provider::KeyProvider::unwrap_for_observation"
RAW = ("encrypt", "decrypt", "encrypt_with_aad", "decrypt_with_aad", "rewrap", "rewrap_with_aad")
AEAD = "aes_gcm::aead::Aead"
INIT = "aes_gcm::aead::KeyInit"
IV_INIT = "cbc::cipher::KeyIvInit"
PDF = "lopdf::Document::encrypt"
GCP = "google_cloud_kms::grpc::apiv1::kms_client::Client"
AWS = "aws_sdk_kms::Client"


def paths(owner, *names):
    return {owner + "::" + name for name in names}


CENSUS_RECORDS = paths(KEYS, "seal_census_record", "open_census_record")
CENSUS_CALLERS = {OBSERVE: CENSUS_OBSERVATION, **dict.fromkeys(CENSUS_RECORDS, CENSUS)}

GCM_TYPES = paths("aes_gcm", "AesGcm", "Aes128Gcm", "Aes256Gcm")
AES_TYPES = {f"aes::Aes{bits}{suffix}" for bits in (128, 192, 256) for suffix in ("", "Enc", "Dec")}
CBC_TYPES = paths("cbc", "Encryptor", "Decryptor")
TYPES = AES_TYPES | GCM_TYPES | CBC_TYPES
INIT_METHODS = paths(INIT, "new", "new_from_slice")
IV_METHODS = paths(IV_INIT, "new", "new_from_slices")
METHODS = (
    paths(KEYS, *RAW) | paths(AES, "encrypt", "decrypt") | {OBSERVE, OBSERVATION_UNWRAP, PDF}
    | CENSUS_RECORDS | INIT_METHODS | IV_METHODS | paths(AEAD, "encrypt", "decrypt")
    | paths(AWS, "encrypt", "decrypt") | paths(GCP, "encrypt", "decrypt")
)

GCM = {"aes_gcm::Aes256Gcm", INIT + "::new_from_slice"}
SEAL = GCM | {AEAD + "::encrypt"}
LARK_ENCRYPT = {"aes::Aes256", "cbc::Encryptor", IV_INIT + "::new"}

# Keys are exact paths under backend/src. Adding a caller requires naming its operation.
REVIEWED = {
    "crypto/aes.rs": paths(KEYS, *RAW) | paths(AES, "encrypt", "decrypt") | SEAL
        | {AEAD + "::decrypt", OBSERVATION_UNWRAP},
    "crypto/aes/record.rs": SEAL | {AEAD + "::decrypt"} | paths(KEYS, "encrypt", "decrypt_with_aad"),
    "crypto/local_key_provider.rs": SEAL | {AEAD + "::decrypt"},
    "crypto/record_field.rs": SEAL | paths(KEYS, "encrypt", "decrypt"),
    "crypto/record_field/historical.rs": paths(KEYS, "encrypt_with_aad", "decrypt_with_aad"),
    "crypto/record_field/historical/tests.rs": SEAL,
    "crypto/record_field/oracle.rs": paths(KEYS, "encrypt_with_aad", "decrypt_with_aad", "rewrap_with_aad"),
    "crypto/record_field/authenticated/tests.rs": SEAL | {OBSERVE} | paths(KEYS, "encrypt", "decrypt"),
    "crypto/aws_kms_provider.rs": paths(AWS, "encrypt", "decrypt"),
    "crypto/gcp_kms_provider.rs": paths(GCP, "encrypt", "decrypt"),
    CENSUS_OBSERVATION + ".rs": {OBSERVE},
    CENSUS + ".rs": CENSUS_RECORDS,
    "services/attachment_extraction.rs": {PDF},
    "services/channel_adapters/lark.rs": LARK_ENCRYPT | {"cbc::Decryptor"},
    "services/platform_native_direct_tests.rs": LARK_ENCRYPT,
    "handlers/nyxbot_media_tests.rs": LARK_ENCRYPT,
}


def reviewed_source(file, definition):
    """Group each census operation under its own reviewed module."""
    if not file.startswith("backend/src/"):
        return None
    source = file[len("backend/src/"):]
    if any(part in ("", ".", "..") for part in source.split("/")):
        return None
    module = CENSUS_CALLERS.get(definition)
    if module and source.startswith(module + "/") and source.endswith(".rs"):
        return module + ".rs"
    return source


def is_reviewed(source, definition):
    if source is None:
        return False
    if definition in CENSUS_CALLERS and source.startswith("crypto/") and source.endswith(".rs"):
        return True
    return definition in REVIEWED.get(source, set())
