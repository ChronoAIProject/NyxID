# BuildKit OCI metadata fixture

Captured from an actual local Docker29.1.3/BuildKit build on the disposable
Linux arm64 validation guest. The image uses pinned
rust:1.98.1-alpine@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f,
the reviewed cargo-auditable0.7.7 musl bootstrap, and a locked synthetic
two-package fixture that prints42. This is an input-format regression fixture,
not NyxID application scan/signature acceptance.

The capture preserves exact JSON blob bytes from streamed docker image save,
including the storage index, Buildx envelope, runnable manifest/config, and
BuildKit auxiliary manifest/config/in-toto statement. Filesystem layers are
omitted; this fixture does not establish layer-content verification. The
embedded BuildKit statement is unsigned and never substitutes for official
GitHub OIDC artifact provenance.

Image index digest:
sha256:2d2742df32b15958e9a12698192ad4c1ac18029d937a7eb36c45d51b6d06a0c3

Capture SHA256:
43d82da6e9d11812d621143d669fa71c4846312c3f53e7457277026b5fdc9c1c
