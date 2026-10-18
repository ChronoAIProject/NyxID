//! Extract bounded audit metadata from the final ELF/Mach-O binary without
//! invoking it. The maintained auditable-info parser validates root uniqueness
//! and graph acyclicity. Source lockfiles are not extraction inputs.
#![forbid(unsafe_code)]

use std::{env, error::Error, fs::File, io::Read, path::PathBuf};

use auditable_info::{Limits, audit_info_from_slice};
use sha2::{Digest, Sha256};

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or("usage: nyxid-artifact-info BINARY")?);
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
    let info = audit_info_from_slice(&bytes, limits.decompressed_json_size)?;
    let receipt = serde_json::json!({
        "schema": "nyxid.rust-binary-info.v1",
        "subject": {"name": path.file_name().ok_or("missing filename")?.to_str().ok_or("non-UTF8 filename")?,
                    "sha256": format!("{:x}", Sha256::digest(&bytes)), "size": bytes.len()},
        "audit": info,
    });
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Artifact metadata extraction failed: {error}");
        std::process::exit(1);
    }
}
