# Logged release provenance fixtures

These are public GitHub Artifact Attestation bundles, retrieved read-only on
2026-10-19. They contain public signing certificates and signatures, not secrets.
The selected attestation is the workflow's logged SLSA provenance, identified by
`verificationMaterial.tlogEntries`. GitHub also returns a release-predicate
attestation with a different identity and RFC3161 evidence; that attestation is
not the workflow provenance accepted by the updater.

| File | Archive/image SHA-256 subject | Bundle file SHA-256 |
| --- | --- | --- |
| `nyxid-v0.15.0.bundle.json` | `7476e161a62e71aeaf1a66ec5d65104a3b1e8e68af2aecf264448d7cde27ee31` | `c468139a80e16ee2410d14a0d488c5b57bcd867ef5385f11ae3b231fb743fc36` |
| `nyxid-v0.40.0.bundle.json` | `b89a8627119c803cbcf40c0c633e01c6e0bda7084afd51476f784b7cd0a820da` | `ada7563ff2663ae22c03d5413b1ba3c88c6a17878bff34be8b8582b62f9b06a9` |
| `nyxid-v0.71.0.bundle.json` | `9b068f9d9361f460987c447d56e10ad1c668821c19b1e23a184c964eebed1f77` | `0516b9bd7b0daabf6361e23bc4f62eac88965be3887e8c7d969d07e872ef7cf9` |
| `nyxid-node-machine-v0.71.0.bundle.json` | `e634854db76b1f7230e79312bab23cb394831a4909920f8a93890878bffe0c1d` | `2a02e9d43bb891ed5617b0196f2e8979be1391916be440b6c4123973b0bb2272` |

Retrieve each from
`https://api.github.com/repos/ChronoAIProject/NyxID/attestations/sha256:<subject>`.
The original bundles all have media type
`application/vnd.dev.sigstore.bundle.v0.3+json` and Rekor `dsse/0.0.1` entries.
Tests that exercise the older promise-only container policy are transformations
of this signed evidence, not claims of an originally published v0.1 fixture.
The historical certificates have expired at verification time; their validity
is checked at the SET-authenticated event time.

The machine fixture binds the published multiarch index
`ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:0.71.0` and the exact
`.github/workflows/publish-images.yml@refs/tags/v0.71.0` certificate identity.
The archive fixtures bind `.github/workflows/release.yml` at their release tags.
The image index was resolved read-only through GHCR's public pull-token endpoint;
its SHA-256 matches the attestation subject. Neither fixture substitutes a
synthetic release certificate for the actual image producer.

`trusted_root.json` is the public test trust material from
[sigstore/root-signing at 63528388118c1002d1221321741159da24c256af](https://github.com/sigstore/root-signing/blob/63528388118c1002d1221321741159da24c256af/targets/trusted_root.json).
Its SHA-256 is
`6494e21ea73fa7ee769f85f57d5a3e6a08725eae1e38c755fc3517c9e6bc0b66`.
This file is only a test fixture. Production loads the target through signed,
expiry-checked TUF metadata rooted in `cli/resources/sigstore-root.json`.

Fixture refreshes must preserve the release identity, subject digest and exact
logged body. Record the retrieval source and new file hashes here. Run both the
CLI and dedicated machine-updater verifier tests, including explicit live TUF
validation, when refreshing producer fixtures or trust bootstrap material.
