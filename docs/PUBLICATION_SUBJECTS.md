# Publication subjects and local inspection

`security/publication-policy.json` declares the complete supported CLI target
and image component/platform sets. Generate the plan before looking at
downloaded files. The CLI generator cross-checks actual cargo-dist output,
Cargo metadata, package version, build matrix and auditable setting. The image
generator cross-checks the actual workflow matrix and Dockerfile ownership.

```sh
dist plan --output-format=json > /tmp/dist-plan.json
python3 scripts/make-publication-plan.py cli --dist-plan /tmp/dist-plan.json --output /tmp/cli-plan.json
python3 scripts/make-publication-plan.py images --output /tmp/images-plan.json
```

The CLI contract has thirteen file subjects: four archives and their checksums,
four global cargo-dist files, and the credential manifest `releases.json`.
The image contract has ten runnable platform manifests and five multi-platform
indices. A reduced downloaded artifact set cannot reduce either contract.
`publication_contract.py` rejects missing, duplicate, extra, stale-commit and
stale-epoch records; index children must match their corresponding inspected
runnable platform digests. Required jobs must all finish with `success`.
Skipped, cancelled and absent jobs fail this check.

Coverage validation is structural. It does not establish valid signatures,
provenance, successful vulnerability scans or policy compliance. The final
producer must run those checks, bind their exact output bytes, and only then
promote staged subjects. These helpers are preparatory; adding them does not
change the current publication workflows into an accepted final gate.

Archive inspection extracts only planned regular assets from bounded archives,
checks the binary format and architecture without executing foreign binaries,
and reads dependency metadata from the actual final binary. Independent Syft
inventory must agree. Both the archive and the extracted binary get distinct
CycloneDX subjects; the archive document includes exact member hashes and the
binary dependency graph. Verify the wizard source seal before inspecting its
presence in the final binary:

```sh
node frontend/scripts/seal-shipped-composition.mjs wizard verify
python3 scripts/check-dist-artifacts.py --plan /tmp/cli-plan.json \
  --artifacts /tmp/downloaded-artifacts --evidence /tmp/archive-evidence \
  --extractor /tmp/tools/nyxid-artifact-info --syft /tmp/tools/syft
python3 scripts/validate-cyclonedx.py /tmp/archive-evidence/*.cdx.json
```

Use the pinned tools and offline Python environment described in
`RUST_ARTIFACT_SBOM.md`. `--target` permits a local subset; `--source-diff`
records a working-tree investigation. Either option marks the result
`local-investigation`, which cannot satisfy complete publication acceptance.
Actual retained binary source-identity enforcement is a separate pending gate;
the expected commit label in a plan is not proof of the binary's built commit.

Buildx may return an index for one platform. That build envelope can contain a
runnable manifest plus an `unknown/unknown` BuildKit attestation manifest.
`oci_subjects.py` preserves the envelope, runnable child and auxiliary digests.
It accepts only the declared Linux platform set (including explicit arm64/v8),
checks config platform/revision, and validates digest/size bindings through
each metadata object. Supported auxiliary descriptors must reference a known
runnable digest and contain an explicitly supported SLSA/SPDX statement bound
to that digest. Unknown, duplicated and unbound descriptors fail. An unsigned
embedded BuildKit statement cannot substitute for official GitHub OIDC
provenance. Layer scanning is separate.

```sh
python3 scripts/check-oci-subjects.py --descriptor /tmp/root-descriptor.json \
  --layout /tmp/oci-layout --platform linux/amd64 --platform linux/arm64 \
  --commit FULL_COMMIT --epoch COMMIT_EPOCH --output /tmp/oci-subjects.json
```

`scripts/fixtures/oci/PROVENANCE.md` records the actual default-isolation
Buildx fixture used by the metadata regression tests. The fixture retains raw
JSON bytes and their digests, and deliberately excludes filesystem layers.
It is metadata-shape evidence, not an application image or signing pass.

Run the bounded negative regressions with:

```sh
python3 scripts/test_publication_contract.py
python3 scripts/test_dist_artifacts.py
python3 scripts/test_oci_subjects.py
```
