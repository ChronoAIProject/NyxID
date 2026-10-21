//! Extract bounded audit metadata from the final ELF/Mach-O binary without
//! invoking it. The maintained auditable-info parser validates root uniqueness
//! and graph acyclicity. Source lockfiles are not extraction inputs.
#![forbid(unsafe_code)]

use std::{env, error::Error, fs::File, io::Read, path::PathBuf};

use auditable_info::{Limits, audit_info_from_slice};
use sha2::{Digest, Sha256};

mod source_identity;

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let first = args
        .next()
        .ok_or("usage: nyxid-artifact-info [--identity-only] BINARY")?;
    let identity_only = first == "--identity-only";
    let path = PathBuf::from(if identity_only {
        args.next().ok_or("missing binary")?
    } else {
        first
    });
    if args.next().is_some() || !path.symlink_metadata()?.is_file() {
        return Err("expected one regular final binary".into());
    }
    let limits = Limits::default();
    let mut bytes = Vec::new();
    File::open(&path)?
        .take(limits.input_file_size as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limits.input_file_size {
        return Err("binary exceeds input bound".into());
    }
    let mut receipt = serde_json::json!({
        "schema": if identity_only { "nyxid.build-identity-receipt.v1" } else { "nyxid.rust-binary-info.v1" },
        "subject": {"name": path.file_name().ok_or("missing filename")?.to_str().ok_or("non-UTF8 filename")?,
                    "sha256": format!("{:x}", Sha256::digest(&bytes)), "size": bytes.len()},
        "build_identity": source_identity::extract(&bytes)?,
    });
    if identity_only {
        if receipt["build_identity"].is_null() {
            return Err("retained build identity is required".into());
        }
    } else {
        receipt["audit"] = serde_json::to_value(audit_info_from_slice(
            &bytes,
            limits.decompressed_json_size,
        )?)?;
    }
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Artifact metadata extraction failed: {error}");
        std::process::exit(1);
    }
}
