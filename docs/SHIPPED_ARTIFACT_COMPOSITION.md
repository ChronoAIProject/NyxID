# Shipped frontend composition

The build records the JavaScript modules that the bundler actually retains,
the sources of retained/generated CSS, and the final bytes served or embedded
in each product. This is a shipped inclusion inventory. The npm lockfile is a
separate source dependency inventory; installing a package does not establish
that its code is shipped. A composition seal is not a signature or an SBOM.

The three products have independent inventories:

| Product | Rendered graph | Final-byte seal |
| --- | --- | --- |
| SPA, including legal pages and credential page | `frontend/dist/composition-graph.json` and the credential graph | `frontend/dist/shipped-composition.json` |
| Standalone backend credential page | `frontend/dist/credential-accept/composition-graph.json` | `frontend/dist/release-integrity/credential-composition.json` |
| CLI embedded wizard | `cli/src/wizard/bundle-meta/rendered-graph.json` | `cli/src/wizard/assets/shipped-composition.json` |

`npm run build` seals after Vite, legal prerendering, the standalone credential
build and the mock-footprint check. `npm run build:credential-accept` also works
on its own, including in the backend image's Node builder. `npm run build:wizard`
captures the code and CSS before singlefile removes their intermediate files,
then seals the actual installed HTML, fonts, licenses and other embedded files.
RustEmbed includes the wizard's seal in the binary alongside those files.

The rendered graph records modules with positive `renderedLength`, their source
SHA256, and exact installed lock path, package name, version, integrity and purl.
`rendered_length` is the bundler's length measure, not a claim about output byte
allocation. Module identities use repository paths or package-relative paths;
they contain no absolute checkout paths. Known Vite/Rolldown runtime helpers
carry their generating tool identity. Unknown virtual modules and packages
without matching installed/locked identities fail the build.

CSS has its own source membership. Retained CSS imports include xterm's
stylesheet and the Tailwind/tw-animate inputs. Tailwind's generating tools are
identified separately. The aggregate CSS output belongs to those sources;
the inventory does not pretend to assign each generated selector a JS module.
Unbundled worklet and legal theme JavaScript must match an exact first-party
source file. Fonts require exact byte hashes, embedded version evidence and a
matching shipped license in `frontend/static-asset-provenance.json`. The font
metadata identifies the vendored bytes; it does not authenticate an upstream
release independently.

Final HTML is parsed using the pinned parse5 tokenizer. Each inline script,
style block and style attribute must match a source template, the exact legal
stylesheet, a recorded Vite template-CSS transformation, or captured singlefile
code. Final local script/stylesheet/modulepreload references must resolve to
accounted output files. Remote stylesheet references require an exact reviewed
URL in `frontend/html-resource-policy.json`; runtime remote bytes are excluded
from shipped-component claims. The parser rejects base elements, event-handler
attributes, srcdoc and javascript URLs, and recognizes case-insensitive rel
tokens. It also checks inert template contents before later instantiation.
Dynamic supported vendor integrations remain governed by the browser policy
and its production tests, as documented in `BROWSER_SECURITY_POLICY.md`.

The sealer validates source identities and output existence, then records every
final file's SHA256 and size. Verification requires the exact file set and
bytes. Added executable files, unmapped inline content, stale mappings, missing
licenses and symlinked/nonregular output files fail. The checked-in wizard
freshness inputs include the parser, provenance policies and sealing scripts.

The release artifact ordering is acyclic:

```text
locked inputs -> rendered graph -> final transforms -> credential seal
    -> SPA seal (includes credential seal)
    -> archive/image bytes -> subject digests + shipped SBOM
    -> detached signatures/attestation bundles -> publication manifest
```

The wizard follows `rendered graph -> singlefile -> installed assets -> wizard
seal -> RustEmbed -> binary -> archive subject digest`. Each seal excludes only
itself from its own inventory. Detached signatures, attestation bundles and
SBOMs belong in a separate release staging directory outside `frontend/dist`
and wizard assets. They must not be injected into the sealed app tree after
sealing: verification rejects a changed file set. Signing a manifest covers its
final bytes; artifact provenance separately binds the final archive/image
subject. Neither signature may be used as a substitute for the other.

With the pinned Node/npm toolchain and locked dependencies installed:

```sh
npm --prefix frontend run build
npm --prefix frontend run build:wizard
node frontend/scripts/seal-shipped-composition.mjs credential-accept verify
node frontend/scripts/seal-shipped-composition.mjs spa verify
node frontend/scripts/seal-shipped-composition.mjs wizard verify
NODE_ENV=test npm --prefix frontend test -- test/shipped-composition.test.ts
```

`frontend/scripts/verify-reproducible-build.py` compares two independent final
SPA/credential builds with identical full commit, commit epoch and toolchain
inputs under thread pools 1 and 8. This is frontend byte repeatability, not
whole-image or cross-platform binary reproducibility. Production policy tests
serve the actual built output through generated nginx configuration; image
extraction parity and full final-image scans remain separate checks.
