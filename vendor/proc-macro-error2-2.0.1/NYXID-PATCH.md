# Rust future compatibility patch

Upstream crate: proc-macro-error2 2.0.1
Source: https://static.crates.io/crates/proc-macro-error2/proc-macro-error2-2.0.1.crate
Original archive SHA-256 (Cargo.lock): 11ec05c52be0a07b08061f7dd003e7d7092e0472bc731b4af7bb1ef876109802
Upstream repository: https://github.com/GnomedDev/proc-macro-error-2
Inspected upstream HEAD b02d79c49898a8f4340aaa81ebd7cae3668ac6a9 still has this defect.

Exact patch: `extern crate proc_macro;` becomes `pub extern crate proc_macro;`.
Rust E0365 (rust-lang/rust#127909) rejects the public reexport of a private
extern crate. This makes its existing intended public visibility explicit;
no warning suppression, macro behavior or dependency changes are included.
The MIT and Apache-2.0 licenses and original Cargo.toml.orig are retained.
All workspace consumers use this crates.io patch. Remove it when a maintained
upstream release includes the correction, with the compiler regression check.
