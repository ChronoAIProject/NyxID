//! Extract identity from its actual retained section, never from an arbitrary
//! string match in a final executable. Foreign targets are parsed, not run.
use serde::{Deserialize, Serialize};
use std::{error::Error, ops::Range};

const START: &[u8] = b"NYXID_BUILD_IDENTITY_V1\0";
const END: &[u8] = b"\0NYXID_BUILD_IDENTITY_END\0";
const LIMIT: usize = 4096;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    schema: String,
    package: String,
    package_version: String,
    target: String,
    commit: String,
    source_date_epoch: Option<u64>,
    source_state: String,
}

fn decode(section: &[u8]) -> Result<Identity, Box<dyn Error>> {
    if section.len() > LIMIT || !section.starts_with(START) || !section.ends_with(END) {
        return Err("invalid retained source identity framing or size".into());
    }
    let identity: Identity =
        serde_json::from_slice(&section[START.len()..section.len() - END.len()])?;
    if identity.schema != "nyxid.build-identity.v1"
        || identity.package.is_empty()
        || !identity
            .package
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        || semver::Version::parse(&identity.package_version).is_err()
        || !matches!(
            identity.target.as_str(),
            "aarch64-apple-darwin"
                | "x86_64-apple-darwin"
                | "aarch64-unknown-linux-gnu"
                | "x86_64-unknown-linux-gnu"
                | "aarch64-unknown-linux-musl"
                | "x86_64-unknown-linux-musl"
        )
    {
        return Err("unsupported retained source identity schema/package/version/target".into());
    }
    if identity.source_state == "unknown" {
        if identity.commit != "unknown" || identity.source_date_epoch.is_some() {
            return Err("inconsistent unknown source identity".into());
        }
    } else if !matches!(
        identity.source_state.as_str(),
        "git-unverified" | "git-dirty" | "exported"
    ) || identity.commit.len() != 40
        || !identity
            .commit
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        || !identity
            .source_date_epoch
            .is_some_and(|epoch| epoch <= 253_402_300_799)
    {
        return Err("invalid retained source commit/state/epoch".into());
    }
    Ok(identity)
}

pub fn extract(bytes: &[u8]) -> Result<Option<Identity>, Box<dyn Error>> {
    let range: Option<Range<usize>> = match binfarce::detect_format(bytes) {
        binfarce::Format::Elf64 {
            byte_order: binfarce::ByteOrder::LittleEndian,
        } => {
            let elf = binfarce::elf64::parse(bytes, binfarce::ByteOrder::LittleEndian)?;
            elf.section_with_name(".nyxid.identity")?
                .map(|section| section.range())
                .transpose()?
        }
        binfarce::Format::Macho if bytes.starts_with(b"\xcf\xfa\xed\xfe") => {
            let macho = binfarce::macho::parse(bytes)?;
            macho
                .section_with_name("__DATA", "__nyxid_id")?
                .map(|section| section.range())
                .transpose()?
        }
        _ => return Err("unsupported executable format for source identity".into()),
    };
    // Duplicate frames, including a second identity section, cannot choose a
    // convenient identity. An unattached string never substitutes for a section.
    let starts = bytes
        .windows(START.len())
        .filter(|window| *window == START)
        .count();
    let ends = bytes
        .windows(END.len())
        .filter(|window| *window == END)
        .count();
    let Some(range) = range else {
        if starts != 0 || ends != 0 {
            return Err("source identity frame outside retained section".into());
        }
        return Ok(None); // Historical tooling subjects; final gate requires it.
    };
    if starts != 1 || ends != 1 {
        return Err("duplicate or missing retained source identity frame".into());
    }
    let identity = decode(
        bytes
            .get(range)
            .ok_or("source identity section exceeds executable")?,
    )?;
    // These checks bind the marker's OS/architecture to the actual container.
    // GNU vs musl additionally requires the executable's loader/ABI inspection.
    let (os, architecture) = if bytes.starts_with(b"\x7fELF") {
        if bytes.len() < 64 || !matches!(u16::from_le_bytes([bytes[16], bytes[17]]), 2 | 3) {
            return Err("source identity requires an ELF executable".into());
        }
        ("linux", u16::from_le_bytes([bytes[18], bytes[19]]) as u32)
    } else {
        if bytes.len() < 32 || u32::from_le_bytes(bytes[12..16].try_into()?) != 2 {
            return Err("source identity requires a Mach-O executable".into());
        }
        ("apple", u32::from_le_bytes(bytes[4..8].try_into()?))
    };
    let expected = match (os, identity.target.split('-').next()) {
        ("linux", Some("aarch64")) => 183,
        ("linux", Some("x86_64")) => 62,
        ("apple", Some("aarch64")) => 0x0100_000c,
        ("apple", Some("x86_64")) => 0x0100_0007,
        _ => return Err("unsupported retained target architecture".into()),
    };
    if architecture != expected || !identity.target.contains(os) {
        return Err("retained target differs from actual executable platform".into());
    }
    Ok(Some(identity))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(value: serde_json::Value) -> Vec<u8> {
        [START, value.to_string().as_bytes(), END].concat()
    }
    fn identity() -> serde_json::Value {
        serde_json::json!({"schema":"nyxid.build-identity.v1","package":"nyxid-cli","package_version":"0.71.0",
            "target":"aarch64-unknown-linux-gnu","commit":"a".repeat(40),"source_date_epoch":1792365742,"source_state":"exported"})
    }
    #[test]
    fn framed_identity_keeps_version_and_source_origin() {
        let value = decode(&frame(identity())).unwrap();
        assert_eq!(value.package_version, "0.71.0");
        assert_eq!(value.source_state, "exported");
        let mut unknown = identity();
        unknown["source_state"] = "unknown".into();
        assert!(decode(&frame(unknown.clone())).is_err());
        unknown["commit"] = "unknown".into();
        unknown["source_date_epoch"] = serde_json::Value::Null;
        assert!(decode(&frame(unknown)).is_ok());
    }
    #[test]
    fn malformed_duplicate_and_unsupported_fields_fail() {
        for (field, value) in [
            ("schema", "other"),
            ("package_version", "garbage"),
            ("commit", "short"),
            ("target", "aarch64-linux"),
            ("source_state", "trusted"),
        ] {
            let mut bad = identity();
            bad[field] = value.into();
            assert!(decode(&frame(bad)).is_err());
        }
        let mut bad = identity();
        bad["unrecognized"] = true.into();
        assert!(decode(&frame(bad)).is_err());
        let duplicate = identity()
            .to_string()
            .replacen('{', "{\"commit\":\"bad\",", 1);
        assert!(decode(&[START, duplicate.as_bytes(), END].concat()).is_err());
        assert!(decode(&[START, &vec![b'x'; LIMIT], END].concat()).is_err());
        assert!(extract(&frame(identity())).is_err());
    }
}
