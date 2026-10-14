use mongodb::{Database, bson::doc};
use quick_xml::{Reader, events::Event};

use crate::{
    errors::{AppError, AppResult},
    models::catalog_icon::{COLLECTION_NAME, CatalogIcon},
};

pub const MAX_SVG_BYTES: usize = 128 * 1024;

/// Only the configured immutable catalog asset route is attachable. Reject
/// alternative UUID encodings, query/fragment suffixes and foreign origins.
pub fn asset_id<'a>(base_url: &str, value: &'a str) -> AppResult<&'a str> {
    let prefix = format!("{}/catalog-icons/", base_url.trim_end_matches('/'));
    let id = value
        .strip_prefix(&prefix)
        .filter(|id| uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == *id))
        .ok_or_else(|| {
            AppError::ValidationError(
                "icon_url must reference an uploaded catalog SVG asset".into(),
            )
        })?;
    Ok(id)
}

pub fn validate(svg: &str) -> AppResult<()> {
    let invalid = || {
        AppError::ValidationError("Use a static SVG with xmlns=\"http://www.w3.org/2000/svg\" and a viewBox; scripts, styles, external resources and animation are unsupported".into())
    };
    if svg.len() > MAX_SVG_BYTES || svg.is_empty() {
        return Err(invalid());
    }
    let mut reader = Reader::from_str(svg);
    let mut depth = 0usize;
    let mut root = false;
    // Browsers render an un-namespaced root as generic XML, which shows as a
    // broken image inside <img>, so the SVG namespace is mandatory.
    let mut root_namespaced = false;
    loop {
        let event = reader.read_event().map_err(|_| invalid())?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.name();
                let tag = std::str::from_utf8(name.as_ref()).map_err(|_| invalid())?;
                if !matches!(
                    tag,
                    "svg"
                        | "g"
                        | "path"
                        | "circle"
                        | "ellipse"
                        | "rect"
                        | "line"
                        | "polyline"
                        | "polygon"
                        | "defs"
                        | "linearGradient"
                        | "radialGradient"
                        | "stop"
                        | "clipPath"
                        | "mask"
                        | "title"
                        | "desc"
                ) {
                    return Err(invalid());
                }
                if depth == 0 {
                    if root || tag != "svg" {
                        return Err(invalid());
                    }
                    root = true;
                    if !e
                        .attributes()
                        .any(|a| a.is_ok_and(|a| a.key.as_ref() == b"viewBox"))
                    {
                        return Err(invalid());
                    }
                }
                for attr in e.attributes() {
                    let attr = attr.map_err(|_| invalid())?;
                    let key = std::str::from_utf8(attr.key.as_ref()).map_err(|_| invalid())?;
                    if !matches!(
                        key,
                        "xmlns"
                            | "viewBox"
                            | "width"
                            | "height"
                            | "id"
                            | "d"
                            | "fill"
                            | "fill-rule"
                            | "fill-opacity"
                            | "stroke"
                            | "stroke-width"
                            | "stroke-linecap"
                            | "stroke-linejoin"
                            | "stroke-miterlimit"
                            | "stroke-dasharray"
                            | "stroke-dashoffset"
                            | "stroke-opacity"
                            | "opacity"
                            | "transform"
                            | "cx"
                            | "cy"
                            | "r"
                            | "rx"
                            | "ry"
                            | "x"
                            | "y"
                            | "x1"
                            | "x2"
                            | "y1"
                            | "y2"
                            | "points"
                            | "offset"
                            | "stop-color"
                            | "stop-opacity"
                            | "gradientUnits"
                            | "gradientTransform"
                            | "spreadMethod"
                            | "fx"
                            | "fy"
                            | "clip-path"
                            | "clip-rule"
                            | "mask"
                            | "maskUnits"
                            | "maskContentUnits"
                            | "preserveAspectRatio"
                    ) {
                        return Err(invalid());
                    }
                    let value = attr
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|_| invalid())?;
                    if key == "viewBox" {
                        let numbers = value
                            .split(|c: char| c.is_ascii_whitespace() || c == ',')
                            .filter(|s| !s.is_empty())
                            .map(str::parse::<f64>)
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(|_| invalid())?;
                        if numbers.len() != 4
                            || numbers.iter().any(|n| !n.is_finite())
                            || numbers[2] <= 0.0
                            || numbers[3] <= 0.0
                        {
                            return Err(invalid());
                        }
                    }
                    let lower = value.to_ascii_lowercase();
                    if key != "xmlns"
                        && (lower.contains(':')
                            || lower.contains('&')
                            || lower.contains('\\')
                            || lower.contains("/*")
                            || (lower.contains("url")
                                && !(lower.starts_with("url(#")
                                    && lower.ends_with(')')
                                    && lower.matches('(').count() == 1)))
                    {
                        return Err(invalid());
                    }
                    if key == "xmlns" {
                        if value != "http://www.w3.org/2000/svg" {
                            return Err(invalid());
                        }
                        if depth == 0 {
                            root_namespaced = true;
                        }
                    }
                }
                if !empty {
                    depth += 1;
                    if depth > 64 {
                        return Err(invalid());
                    }
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or_else(invalid)?;
            }
            Event::DocType(_) | Event::PI(_) | Event::CData(_) | Event::GeneralRef(_) => {
                return Err(invalid());
            }
            Event::Text(t) if depth == 0 && !t.iter().all(u8::is_ascii_whitespace) => {
                return Err(invalid());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !root || !root_namespaced || depth != 0 {
        return Err(invalid());
    }
    Ok(())
}

pub async fn store(db: &Database, svg: &str) -> AppResult<String> {
    validate(svg)?;
    let id = uuid::Uuid::new_v4().to_string();
    db.collection::<CatalogIcon>(COLLECTION_NAME)
        .insert_one(CatalogIcon {
            id: id.clone(),
            svg: svg.to_owned(),
        })
        .await?;
    Ok(id)
}

pub async fn load(db: &Database, id: &str) -> AppResult<String> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(AppError::NotFound("Icon not found".into()));
    }
    db.collection::<CatalogIcon>(COLLECTION_NAME)
        .find_one(doc! {"_id": id})
        .await?
        .map(|i| i.svg)
        .ok_or_else(|| AppError::NotFound("Icon not found".into()))
}

/// Immutable assets must exist before metadata can reference them. Session reads
/// bind manager attachment to the same snapshot as authority and metadata.
pub async fn require_asset(
    db: &Database,
    id: &str,
    session: Option<&mut mongodb::ClientSession>,
) -> AppResult<()> {
    let coll = db.collection::<CatalogIcon>(COLLECTION_NAME);
    let query = coll.find_one(doc! {"_id": id});
    let asset = match session {
        Some(session) => query.session(session).await?,
        None => query.await?,
    };
    if asset.is_none() {
        return Err(AppError::ValidationError(
            "icon_url must reference an existing uploaded catalog SVG asset".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asset_references_require_exact_origin_prefix_and_canonical_identity() {
        let id = uuid::Uuid::new_v4().to_string();
        let base = "https://nyxid.example";
        let url = format!("{base}/catalog-icons/{id}");
        assert_eq!(asset_id(base, &url).unwrap(), id);
        for invalid in [
            format!("{url}?x=1"),
            format!("{url}#x"),
            format!("{url}/extra"),
            format!("https://foreign.example/catalog-icons/{id}"),
            format!("{base}/other/catalog-icons/{id}"),
            format!("{base}/catalog-icons/{}", id.replace('-', "")),
            format!("{base}/catalog-icons/{}", id.to_uppercase()),
        ] {
            assert!(asset_id(base, &invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn accepts_static_logo() {
        assert!(validate(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#abc" d="M0 0h24v24z"/></svg>"##).is_ok());
    }
    #[test]
    fn rejects_active_or_external_content() {
        for body in [
            "<script/>",
            "<foreignObject/>",
            "<image href=\"https://example.com/a\"/>",
            "<path onclick=\"alert(1)\"/>",
            "<path style=\"fill:red\"/>",
            "<path fill=\"url(https://example.com/a)\"/>",
        ] {
            assert!(validate(&format!("<svg viewBox=\"0 0 24 24\">{body}</svg>")).is_err());
        }
        assert!(validate("<!DOCTYPE svg><svg viewBox=\"0 0 1 1\"/>").is_err());
        assert!(
            validate("<svg viewBox=\"0 0 1 1\"/>").is_err(),
            "an SVG without the SVG namespace renders as a broken image"
        );
        assert!(validate("<svg/>").is_err());
        assert!(validate("<svg viewBox=\"0 0 1 1\"><path></svg>").is_err());
        assert!(validate("<svg viewBox=\"0 0 1 1\"/><svg viewBox=\"0 0 1 1\"/>").is_err());
        assert!(
            validate(
                r#"<svg viewBox="0 0 1 1"><path fill="u&#114;l(https://example.com)"/></svg>"#
            )
            .is_err()
        );
        assert!(validate(&"x".repeat(MAX_SVG_BYTES + 1)).is_err());
    }
}
