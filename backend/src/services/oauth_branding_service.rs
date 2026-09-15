//! Branding assets are decoded and re-encoded locally, never fetched from a URL.
//! Immutable PNG pixels live in GridFS; advisory homepages are shape-validated only.

use futures::io::{AsyncReadExt, AsyncWriteExt};
use image::{ImageFormat, ImageReader, Limits};
use mongodb::{
    Database,
    bson::{Bson, Document, doc},
    gridfs::GridFsBucket,
    options::GridFsBucketOptions,
};
use std::io::Cursor;

use crate::errors::{AppError, AppResult};
use crate::models::oauth_client::{COLLECTION_NAME, OauthClient};

pub const MAX_LOGO_INPUT: usize = 256 * 1024;
const MAX_LOGO_OUTPUT: u64 = 2 * 1024 * 1024;

pub fn logo_url(client: &OauthClient) -> Option<String> {
    client
        .is_active
        .then_some(client.logo_asset_id.as_ref())
        .flatten()
        .map(|id| format!("/api/v1/branding/assets/{id}"))
}

pub fn is_verified(client: &OauthClient) -> bool {
    client.is_active && client.branding_verified_revision == Some(client.branding_revision)
}

pub fn destination(client: &OauthClient, redirect_uri: &str) -> AppResult<String> {
    let uri = url::Url::parse(redirect_uri).map_err(|_| AppError::InvalidRedirectUri)?;
    Ok(if matches!(uri.scheme(), "http" | "https") {
        uri.host_str().unwrap_or_default().into()
    } else {
        format!("desktop app registered as {}", client.client_name)
    })
}

pub fn validate_homepage(value: &str) -> AppResult<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let invalid =
        || AppError::ValidationError("Homepage must be an HTTPS URL with a public host".into());
    if value.len() > 2048 {
        return Err(invalid());
    }
    let url = url::Url::parse(value).map_err(|_| invalid())?;
    super::url_validation::reject_url_userinfo(&url)?;
    if url.scheme() != "https" {
        return Err(invalid());
    }
    match url.host().ok_or_else(invalid)? {
        url::Host::Ipv4(ip) if super::url_validation::is_private_or_internal_ip(ip.into()) => {
            return Err(invalid());
        }
        url::Host::Ipv6(ip) if super::url_validation::is_private_or_internal_ip(ip.into()) => {
            return Err(invalid());
        }
        url::Host::Domain(host) => {
            let host = host.trim_end_matches('.');
            if !host.contains('.')
                || ["localhost", "local", "internal", "test", "invalid"]
                    .iter()
                    .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
            {
                return Err(invalid());
            }
        }
        _ => {}
    }
    Ok(Some(url.to_string()))
}

/// Compare the old branding and update its revision atomically with the fields.
/// All ordinary values are literals so a name beginning with '$' stays text.
pub fn update_pipeline(fields: Document) -> Vec<Document> {
    let changes = [
        "client_name",
        "handoff_blurb",
        "homepage_url",
        "logo_asset_id",
    ]
    .iter()
    .filter_map(|field| {
        fields.get(*field).map(|value| Bson::Document(doc! {
            "$ne": [{ "$ifNull": [format!("${field}"), Bson::Null] }, { "$literal": value.clone() }]
        }))
    })
    .collect::<Vec<_>>();
    let mut set = Document::new();
    for (key, value) in fields {
        set.insert(key, doc! { "$literal": value });
    }
    if !changes.is_empty() {
        set.insert("branding_revision", doc! {
            "$add": [{ "$ifNull": ["$branding_revision", 0] }, { "$cond": [{ "$or": changes }, 1, 0] }]
        });
    }
    vec![doc! { "$set": set }]
}

pub fn reencode_logo(bytes: &[u8]) -> AppResult<Vec<u8>> {
    let invalid = || {
        AppError::ValidationError(
            "Logo must be a PNG or WebP image, at most 256 KiB and 512 by 512 pixels".into(),
        )
    };
    if bytes.is_empty() || bytes.len() > MAX_LOGO_INPUT {
        return Err(invalid());
    }
    let format = image::guess_format(bytes).map_err(|_| invalid())?;
    if !matches!(format, ImageFormat::Png | ImageFormat::WebP) {
        return Err(invalid());
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| invalid())?;
    if image.width() == 0 || image.height() == 0 || image.width() > 512 || image.height() > 512 {
        return Err(invalid());
    }
    let pixels = image::DynamicImage::ImageRgba8(image.into_rgba8());
    let mut output = Cursor::new(Vec::new());
    pixels
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| invalid())?;
    Ok(output.into_inner())
}

fn bucket(db: &Database) -> GridFsBucket {
    db.gridfs_bucket(
        GridFsBucketOptions::builder()
            .bucket_name("branding_assets".to_string())
            .build(),
    )
}

pub async fn upload_logo(
    db: &Database,
    client_id: &str,
    owner: &str,
    bytes: Vec<u8>,
) -> AppResult<OauthClient> {
    let png = tokio::task::spawn_blocking(move || reencode_logo(&bytes))
        .await
        .map_err(|_| AppError::Internal("Logo decoding task failed".into()))??;
    let id = uuid::Uuid::new_v4().to_string();
    let bucket = bucket(db);
    let mut stream = bucket
        .open_upload_stream("logo.png")
        .id(Bson::String(id.clone()))
        .await?;
    stream
        .write_all(&png)
        .await
        .map_err(|_| AppError::Internal("Could not store logo".into()))?;
    stream
        .close()
        .await
        .map_err(|_| AppError::Internal("Could not finish storing logo".into()))?;
    let result = db
        .collection::<OauthClient>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": client_id, "created_by": owner, "is_active": true },
            update_pipeline(doc! { "logo_asset_id": &id, "updated_at": bson::DateTime::now() }),
        )
        .await;
    match result {
        Ok(result) if result.matched_count == 1 => {
            super::oauth_client_service::get_client(db, client_id).await
        }
        other => {
            let _ = bucket.delete(Bson::String(id)).await;
            match other {
                Err(error) => Err(error.into()),
                _ => Err(AppError::NotFound("OAuth client not found".into())),
            }
        }
    }
}

pub async fn read_logo(db: &Database, id: &str) -> AppResult<Vec<u8>> {
    let not_found = || AppError::NotFound("Branding asset not found".into());
    uuid::Uuid::parse_str(id).map_err(|_| not_found())?;
    if db
        .collection::<Document>("branding_assets.files")
        .find_one(doc! { "_id": id })
        .await?
        .is_none()
    {
        return Err(not_found());
    }
    let mut stream = bucket(db)
        .open_download_stream(Bson::String(id.into()))
        .await?
        .take(MAX_LOGO_OUTPUT + 1);
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| AppError::Internal("Could not read logo".into()))?;
    if bytes.len() as u64 > MAX_LOGO_OUTPUT {
        return Err(AppError::Internal("Stored logo exceeds limit".into()));
    }
    Ok(bytes)
}

pub async fn verify(
    db: &Database,
    client_id: &str,
    revision: u32,
    verified: bool,
) -> AppResult<OauthClient> {
    let result = db.collection::<OauthClient>(COLLECTION_NAME).update_one(
        doc! { "_id": client_id, "$expr": { "$eq": [{ "$ifNull": ["$branding_revision", 0] }, revision] } },
        doc! { "$set": { "branding_verified_revision": verified.then_some(revision), "updated_at": bson::DateTime::now() } },
    ).await?;
    if result.matched_count == 0 {
        return Err(AppError::BadRequest(
            "Branding changed; reload before verifying".into(),
        ));
    }
    super::oauth_client_service::get_client(db, client_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ExtendedColorType, ImageEncoder};

    pub(crate) fn png() -> Vec<u8> {
        let mut data = Vec::new();
        image::codecs::png::PngEncoder::new(&mut data)
            .write_image(&[255, 0, 0, 255], 1, 1, ExtendedColorType::Rgba8)
            .unwrap();
        data
    }

    #[test]
    fn logo_rejects_svg_non_images_oversize_and_large_dimensions() {
        for bytes in [
            b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
            b"not an image".to_vec(),
            vec![0; MAX_LOGO_INPUT + 1],
        ] {
            assert!(reencode_logo(&bytes).is_err());
        }
        let mut big = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(513, 1)
            .write_to(&mut big, ImageFormat::Png)
            .unwrap();
        assert!(reencode_logo(big.get_ref()).is_err());
        assert!(reencode_logo(&png()).is_ok());
    }

    #[test]
    fn logo_reencodes_webp_and_strips_png_metadata() {
        let mut webp = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
            .write_image(&[255, 0, 0, 255], 1, 1, ExtendedColorType::Rgba8)
            .unwrap();
        assert_eq!(
            image::guess_format(&reencode_logo(&webp).unwrap()).unwrap(),
            ImageFormat::Png
        );
        let mut source = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut source);
        encoder
            .set_exif_metadata(b"private-camera-metadata".to_vec())
            .unwrap();
        encoder
            .set_icc_profile(b"private-color-profile".to_vec())
            .unwrap();
        encoder
            .write_image(&[255, 0, 0, 255], 1, 1, ExtendedColorType::Rgba8)
            .unwrap();
        assert!(source.windows(4).any(|b| b == b"eXIf"));
        let clean = reencode_logo(&source).unwrap();
        let mut offset = 8;
        while offset < clean.len() {
            let length = u32::from_be_bytes(clean[offset..offset + 4].try_into().unwrap()) as usize;
            let kind = &clean[offset + 4..offset + 8];
            assert!([b"IHDR", b"IDAT", b"IEND"].contains(&kind.try_into().unwrap()));
            offset += 12 + length;
        }
        assert_eq!(
            image::load_from_memory(&clean)
                .unwrap()
                .into_rgba8()
                .as_raw(),
            &[255, 0, 0, 255]
        );
    }

    #[test]
    fn homepage_shape_validation_never_resolves_or_fetches() {
        assert_eq!(
            validate_homepage("https://nonexistent-brand.example.com/about")
                .unwrap()
                .as_deref(),
            Some("https://nonexistent-brand.example.com/about")
        );
        assert_eq!(validate_homepage("").unwrap(), None);
        for url in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://localhost",
            "https://127.0.0.1",
            "https://169.254.169.254",
            "https://[::1]",
            "https://host.internal",
            "https://internal",
        ] {
            assert!(validate_homepage(url).is_err(), "{url}");
        }
    }
}
