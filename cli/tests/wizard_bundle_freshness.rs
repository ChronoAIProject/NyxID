//! Verifies that the committed wizard bundle matches the current source
//! closure without rebuilding in CI. Replaces the byte-exact rebuild
//! drift check (fragile against Vite / esbuild version drift) and the
//! hand-maintained regex touch-check (misses transitive imports like
//! `components/ui/**`, `hooks/**`, `lib/api-client.ts`).
//!
//! This check folds into the existing `cli-test` CI job so no separate
//! workflow step is needed.
//!
//! The hashing algorithm here MUST stay in lockstep with the Node
//! install script at `frontend/scripts/install-wizard-bundle.mjs`. Any
//! drift between the two means `cargo test` produces a hash that can
//! never match what `npm run build:wizard` wrote.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const EXTRAS: &[&str] = &[
    "frontend/package-lock.json",
    "frontend/vite.wizard.config.ts",
    "frontend/wizard.html",
    "frontend/vite-plugins/wizard-manifest.ts",
    ".node-version",
];

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<repo>/cli`; parent is the repo root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CARGO_MANIFEST_DIR has no parent")
        .to_path_buf()
}

fn read_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| {
        panic!(
            "wizard freshness: failed to read {}: {e}\n\
             if this file was just moved or deleted, rebuild the bundle:\n  \
             npm --prefix frontend run build:wizard",
            path.display(),
        )
    })
}

#[test]
fn wizard_bundle_is_fresh() {
    let repo = repo_root();
    let meta_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("wizard")
        .join("bundle-meta");
    let manifest_path = meta_dir.join("index.manifest");
    let hash_path = meta_dir.join("index.hash");

    let manifest_bytes = read_bytes(&manifest_path);
    let manifest_text = std::str::from_utf8(&manifest_bytes)
        .expect("wizard freshness: manifest is not valid UTF-8");

    let mut hasher = Sha256::new();

    for file in manifest_text.lines().filter(|l| !l.is_empty()) {
        let abs = repo.join(file);
        hasher.update(file.as_bytes());
        hasher.update([0u8]);
        hasher.update(read_bytes(&abs));
        hasher.update([0u8]);
    }
    for file in EXTRAS {
        let abs = repo.join(file);
        hasher.update(file.as_bytes());
        hasher.update([0u8]);
        hasher.update(read_bytes(&abs));
        hasher.update([0u8]);
    }
    hasher.update(&manifest_bytes);

    let computed = hex::encode(hasher.finalize());
    let committed = fs::read_to_string(&hash_path)
        .unwrap_or_else(|e| {
            panic!(
                "wizard freshness: failed to read {}: {e}\n\
                 rebuild the bundle with:\n  \
                 npm --prefix frontend run build:wizard",
                hash_path.display(),
            )
        })
        .trim()
        .to_string();

    assert_eq!(
        computed, committed,
        "\n\
         ============================================================\n\
         wizard bundle is stale — source closure hash mismatch\n\
         ============================================================\n\
         expected (from source): {computed}\n\
         committed:              {committed}\n\n\
         rebuild and commit:\n  \
         npm --prefix frontend run build:wizard\n  \
         git add cli/src/wizard/\n\
         ============================================================\n",
    );
}
