//! Validated RSA import and fail-closed file creation. PEM contents never enter
//! diagnostics. A surviving member of a partial pair is never overwritten.
use super::*;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use std::io::Write;
use zeroize::Zeroizing;

pub(super) fn parse_public(pem: &str) -> Result<RsaPublicKey, AppError> {
    RsaPublicKey::from_pkcs1_pem(pem)
        .or_else(|_| RsaPublicKey::from_public_key_pem(pem))
        .map_err(|_| AppError::Internal("Invalid RSA public key PEM".into()))
}

pub(super) fn load(private_path: &Path, public_path: &Path) -> Result<JwtKeys, AppError> {
    let private_pem = Zeroizing::new(
        fs::read_to_string(private_path)
            .map_err(|_| AppError::Internal("Failed to read RSA private key".into()))?,
    );
    let public_pem = fs::read_to_string(public_path)
        .map_err(|_| AppError::Internal("Failed to read RSA public key".into()))?;
    let private = RsaPrivateKey::from_pkcs1_pem(&private_pem)
        .or_else(|_| RsaPrivateKey::from_pkcs8_pem(&private_pem))
        .map_err(|_| AppError::Internal("Invalid RSA private key PEM".into()))?;
    private
        .validate()
        .map_err(|_| AppError::Internal("Invalid RSA private key parameters".into()))?;
    let public = parse_public(&public_pem)?;
    if public.n().bits() < 3072 || private.n().bits() < 3072 {
        return Err(AppError::Internal(
            "JWT RSA keys require at least 3072 bits".into(),
        ));
    }
    if private.to_public_key() != public {
        return Err(AppError::Internal(
            "JWT RSA private/public keys do not match".into(),
        ));
    }
    Ok(JwtKeys {
        encoding: EncodingKey::from_rsa_pem(private_pem.as_bytes())
            .map_err(|_| AppError::Internal("Invalid JWT encoding key".into()))?,
        decoding: DecodingKey::from_rsa_pem(public_pem.as_bytes())
            .map_err(|_| AppError::Internal("Invalid JWT verification key".into()))?,
        kid: hex::encode(&Sha256::digest(public.n().to_bytes_be())[..8]),
        retained: Default::default(),
        public_jwk: Some(public_key_jwk(&public_pem)?),
    })
}

pub(super) fn create_pair(private_path: &Path, public_path: &Path) -> Result<(), AppError> {
    // Check before expensive generation; create_new below is the authoritative
    // race/symlink fence. Distinct files are required even for fresh paths.
    if private_path == public_path || private_path.exists() || public_path.exists() {
        return Err(AppError::Internal(
            "Refusing to replace existing RSA key files".into(),
        ));
    }
    for path in [private_path, public_path] {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|_| AppError::Internal("Failed to create RSA key directory".into()))?;
        }
    }
    let private = RsaPrivateKey::new(&mut rand::thread_rng(), 4096)
        .map_err(|_| AppError::Internal("RSA key generation failed".into()))?;
    let private_pem = private
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .map_err(|_| AppError::Internal("Failed to encode RSA private key".into()))?;
    let public_pem = private
        .to_public_key()
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .map_err(|_| AppError::Internal("Failed to encode RSA public key".into()))?;
    let mut private_file = create_restricted(private_path)?;
    // A collision here leaves a partial, empty pair requiring operator action;
    // it never truncates the file another process owns or silently rotates keys.
    let mut public_file = create_restricted(public_path)?;
    private_file
        .write_all(private_pem.as_bytes())
        .and_then(|_| private_file.sync_all())
        .map_err(|_| AppError::Internal("Failed to persist RSA private key".into()))?;
    public_file
        .write_all(public_pem.as_bytes())
        .and_then(|_| public_file.sync_all())
        .map_err(|_| AppError::Internal("Failed to persist RSA public key".into()))?;
    Ok(())
}

fn create_restricted(path: &Path) -> Result<fs::File, AppError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|_| AppError::Internal("Failed to create new RSA key file".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn write_pair(dir: &Path, bits: usize) -> (std::path::PathBuf, std::path::PathBuf) {
        let private = RsaPrivateKey::new(&mut rand::thread_rng(), bits).unwrap();
        let private_path = dir.join("private.pem");
        let public_path = dir.join("public.pem");
        fs::write(
            &private_path,
            private
                .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        fs::write(
            &public_path,
            private
                .to_public_key()
                .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
                .unwrap(),
        )
        .unwrap();
        (private_path, public_path)
    }
    #[test]
    fn import_rejects_weak_mismatched_and_invalid_material_and_accepts_strong_pair() {
        let dir = tempfile::tempdir().unwrap();
        let (private, public) = write_pair(dir.path(), 2048);
        let mut config = crate::test_utils::test_app_config();
        config.jwt_private_key_path = private.to_str().unwrap().into();
        config.jwt_public_key_path = public.to_str().unwrap().into();
        assert!(
            JwtKeys::from_config(&config)
                .err()
                .unwrap()
                .to_string()
                .contains("at least 3072 bits")
        );
        let (private, public) = write_pair(dir.path(), 3072);
        let keys = JwtKeys::from_config(&config).unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(keys.kid.clone());
        let token = encode(
            &header,
            &serde_json::json!({"exp":Utc::now().timestamp()+60}),
            &keys.encoding,
        )
        .unwrap();
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_aud = false;
        decode::<serde_json::Value>(&token, &keys.decoding, &validation).unwrap();
        let other = tempfile::tempdir().unwrap();
        let (_, other_public) = write_pair(other.path(), 3072);
        fs::copy(other_public, &public).unwrap();
        assert!(
            JwtKeys::from_config(&config)
                .err()
                .unwrap()
                .to_string()
                .contains("do not match")
        );
        fs::write(&private, "distinctive-secret-invalid-private-pem").unwrap();
        let error = load(&private, &public).err().unwrap();
        assert!(!format!("{error:?} {error}").contains("distinctive-secret"));
    }
    #[test]
    fn keyring_overlap_selects_retained_keys_for_access_refresh_id_and_relay_then_retires() {
        let old_dir = tempfile::tempdir().unwrap();
        let new_dir = tempfile::tempdir().unwrap();
        let (old_private, old_public) = write_pair(old_dir.path(), 3072);
        let (new_private, new_public) = write_pair(new_dir.path(), 3072);
        let old = load(&old_private, &old_public).unwrap();
        let mut current = load(&new_private, &new_public).unwrap();
        current
            .load_retained(&[old_public.to_str().unwrap().into()])
            .unwrap();
        let config = crate::test_utils::test_app_config();
        let user = Uuid::new_v4();
        let access =
            generate_access_token(&old, &config, &user, "openid", None, None, None, None, None)
                .unwrap();
        let refresh = generate_refresh_token(&old, &config, &user).unwrap().0;
        let delegation =
            generate_delegated_access_token(&old, &config, &user, "proxy:*", "service", 300, None)
                .unwrap();
        let relay = generate_relay_access_token(
            &old,
            &config,
            &user,
            "proxy",
            None,
            &RelayAgentScope {
                api_key_id: "overlap-agent".into(),
                api_key_name: "Agent".into(),
                allowed_service_ids: vec!["service".into()],
                allowed_node_ids: vec![],
                allow_all_services: false,
                allow_all_nodes: true,
            },
        )
        .unwrap();
        for token in [&access, &refresh, &delegation, &relay] {
            assert_eq!(
                verify_token(&current, &config, token).unwrap().sub,
                user.to_string()
            );
        }
        let reply =
            generate_relay_reply_token(&old, &config, "key", "conversation", "message", "lark")
                .unwrap();
        validate_relay_reply_token(&current, &config, &reply).unwrap();
        let callback = generate_relay_callback_token(
            &old,
            &config,
            "jti",
            "key",
            "message",
            "lark",
            &hex::encode(Sha256::digest(b"body")),
        )
        .unwrap();
        validate_relay_callback_token(&current, &config, &callback).unwrap();
        let id = generate_id_token(
            &old,
            &config,
            &user,
            Some("overlap@example.test"),
            Some(true),
            None,
            None,
            "relying-party",
            Some("overlap-nonce"),
            Some(&access),
            None,
        )
        .unwrap();
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&["relying-party"]);
        let identity =
            decode::<IdTokenClaims>(&id, current.decoding_key(&id).unwrap(), &validation)
                .unwrap()
                .claims;
        assert_eq!(identity.sub, user.to_string());
        assert_eq!(identity.nonce.as_deref(), Some("overlap-nonce"));
        let new_access = generate_access_token(
            &current, &config, &user, "openid", None, None, None, None, None,
        )
        .unwrap();
        assert_eq!(
            jsonwebtoken::decode_header(&new_access)
                .unwrap()
                .kid
                .as_deref(),
            Some(current.kid.as_str())
        );
        let jwks = current.public_keys(&serde_json::Value::Null);
        assert_eq!(jwks.len(), 2);
        assert!(jwks.iter().any(|key| key["kid"] == old.kid));
        for kid in [None, Some("unknown-key".to_string())] {
            let mut header = Header::new(Algorithm::RS256);
            header.kid = kid;
            let token = encode(
                &header,
                &serde_json::json!({"exp":Utc::now().timestamp()+60}),
                &current.encoding,
            )
            .unwrap();
            assert!(current.decoding_key(&token).is_err());
        }
        let hs = encode(
            &Header::new(Algorithm::HS256),
            &serde_json::json!({"exp":Utc::now().timestamp()+60}),
            &EncodingKey::from_secret(b"not-rsa"),
        )
        .unwrap();
        assert!(current.decoding_key(&hs).is_err());
        assert!(
            current
                .load_retained(&[new_public.to_str().unwrap().into()])
                .is_err()
        );
        // A failed reload leaves the previously validated ring intact.
        assert!(verify_token(&current, &config, &access).is_ok());
        current.load_retained(&[]).unwrap();
        for token in [&access, &refresh, &delegation, &relay] {
            assert!(verify_token(&current, &config, token).is_err());
        }
        assert!(validate_relay_reply_token(&current, &config, &reply).is_err());
        assert!(validate_relay_callback_token(&current, &config, &callback).is_err());
        assert!(current.decoding_key(&id).is_err());
        assert!(verify_token(&current, &config, &new_access).is_ok());
        assert_eq!(current.public_keys(&serde_json::Value::Null).len(), 1);
    }

    #[test]
    fn partial_development_pair_is_rejected_without_overwriting_survivor() {
        for surviving_private in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let private = dir.path().join("private.pem");
            let public = dir.path().join("public.pem");
            let survivor = if surviving_private { &private } else { &public };
            fs::write(survivor, "distinctive-surviving-key-file").unwrap();
            let mut config = crate::test_utils::test_app_config();
            config.jwt_private_key_path = private.to_str().unwrap().into();
            config.jwt_public_key_path = public.to_str().unwrap().into();
            assert!(JwtKeys::from_config(&config).is_err());
            assert_eq!(
                fs::read_to_string(survivor).unwrap(),
                "distinctive-surviving-key-file"
            );
            assert!(!if surviving_private { public } else { private }.exists());
        }
    }
    #[test]
    fn restricted_creation_never_truncates_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private.pem");
        let mut file = create_restricted(&path).unwrap();
        file.write_all(b"surviving-key").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(create_restricted(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"surviving-key");
    }
}
