use mongodb::{Database, bson::doc};
use quick_xml::{Reader, events::Event};

use crate::{
    errors::{AppError, AppResult},
    models::catalog_icon::{COLLECTION_NAME, CatalogIcon},
};

pub const MAX_SVG_BYTES: usize = 128 * 1024;

pub fn validate(svg: &str) -> AppResult<()> {
    let invalid = || {
        AppError::ValidationError("Use a static SVG with a viewBox; scripts, styles, external resources and animation are unsupported".into())
    };
    if svg.len() > MAX_SVG_BYTES || svg.is_empty() {
        return Err(invalid());
    }
    let mut reader = Reader::from_str(svg);
    let mut depth = 0usize;
    let mut root = false;
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
                    if key == "xmlns" && value != "http://www.w3.org/2000/svg" {
                        return Err(invalid());
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
    if !root || depth != 0 {
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

#[cfg(test)]
mod tests {
    use super::*;
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
