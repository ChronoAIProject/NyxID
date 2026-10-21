#!/usr/bin/env python3
"""Compile the precise E0365 regression with the real installed Rust compiler.

Use a disposable target directory outside the repository. This small standalone
consumer also avoids Cargo 1.98.1's panic when selecting a host-only dependency
directly with `-p` in this resolver-3 workspace.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", type=Path, required=True)
    args = parser.parse_args()
    args.evidence_dir.mkdir(parents=True, exist_ok=True)
    evidence = args.evidence_dir.resolve()
    env = dict(os.environ, CARGO_BUILD_JOBS="2", CARGO_TARGET_DIR=str(evidence / "target"), RUSTFLAGS="-Dpub_use_of_private_extern_crate")
    with tempfile.TemporaryDirectory(prefix="nyxid-proc-macro-", dir=evidence) as temporary:
        fixture = Path(temporary)
        (fixture / "src").mkdir()
        (fixture / "src/lib.rs").write_text("pub fn fixture() {}\n")
        for variant in ("original", "corrected"):
            dependency = fixture / variant
            shutil.copytree(ROOT / "vendor/proc-macro-error2-2.0.1", dependency)
            if variant == "original":
                path = dependency / "src/lib.rs"
                text = path.read_text()
                assert text.count("pub extern crate proc_macro;") == 1
                path.write_text(text.replace("pub extern crate proc_macro;", "extern crate proc_macro;"))
            (fixture / "Cargo.toml").write_text('[package]\nname="nyxid-proc-macro-regression"\nversion="0.0.0"\nedition="2024"\n[workspace]\nexclude=["original","corrected"]\n[dependencies]\nproc-macro-error2={path="' + variant + '"}\n')
            subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=fixture, env=env, check=True)
            result = subprocess.run(["cargo", "check", "--locked", "--offline", "--future-incompat-report"], cwd=fixture, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            (evidence / (variant + ".log")).write_text(result.stdout)
            if variant == "original":
                if result.returncode == 0 or "E0365" not in result.stdout:
                    raise SystemExit("original compiler regression did not reject E0365")
            elif result.returncode != 0 or "E0365" in result.stdout:
                raise SystemExit("corrected compiler regression failed: " + result.stdout)
    print("Original E0365 rejected; corrected crate accepted with future-compatibility lint denied")


if __name__ == "__main__":
    main()
