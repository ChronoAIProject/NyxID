# Releasing NyxID

This document covers the release path for the `nyxid` CLI binaries produced by CI.

## Codex Onboarding Rollout

Ship the Codex recommendation only with a release that provides
`nyxid provider connect-codex --help` and canonical `~/.agents/skills` installation.
Release `v0.16.0` introduces these capabilities; the original published `v0.15.0`
predates them. Do not infer availability from version text
or replace an existing published tag. Deploy the compatible provider-connection
backend routes before the landing prompt; verify installer -> skill registration
-> optional consent -> saved connection verification using isolated fixtures.
Older releases must offer upgrade or separate provider authorization, never
manual credential inspection. Installation must remain successful when skipped.

## Release Trigger

Releases are tag-driven. From an up-to-date `main` checkout:

```bash
git pull --ff-only origin main
git tag vX.Y.Z
git push origin vX.Y.Z
```

Use the same `X.Y.Z` version as `cli/Cargo.toml`. Do not reuse a published tag for different bits; cut a new patch tag if the release contents need to change.

## What CI Builds

The `Release` workflow runs cargo-dist and publishes the `nyxid-cli` package for:

- `x86_64-unknown-linux-gnu` on `ubuntu-24.04`
- `aarch64-unknown-linux-gnu` on `ubuntu-24.04-arm` inside an Ubuntu 20.04-based arm64 container, with cargo-dist metadata set to `glibc 2.31`
- `x86_64-apple-darwin` on `macos-latest`
- `aarch64-apple-darwin` on `macos-latest`

Windows is not yet a release target. `pty-process`, `nix`, and parts of `rustix` used by the SSH/PTY paths in `cli/src/node/ws_client.rs` are Unix-only and aren't yet `cfg(unix)`-gated. Until that audit lands, Windows users should run `nyxid` under WSL — see [docs/WINDOWS_SETUP.md](WINDOWS_SETUP.md).

Linux release runners install `libdbus-1-dev` and `pkg-config` because the CLI uses the keyring crate's Secret Service backend on Linux. The Linux arm64 GNU artifact is built in the Ubuntu 20.04 container so stock Jetson-class Ubuntu 20.04 hosts with `glibc 2.31` are inside the advertised ABI baseline. That arm64 release job also installs `clang` and exports `CC=clang` / `CXX=clang++` for the artifact build so C dependencies such as `aws-lc-sys` do not use the Ubuntu 20.04 GCC 9.x compiler path. The wizard bundle is already committed under `cli/src/wizard/assets/`, so release builds do not need Node.

cargo-dist publishes:

- Platform archives: `nyxid-cli-<target>.tar.gz`
- SHA-256 checksums
- `nyxid-cli-installer.sh`
- Generated GitHub release notes, with NyxID changelog and container image notes appended by the workflow

## Attestations

The release workflow uses GitHub Artifact Attestations with Sigstore-backed keyless signing. There are no minisign keys, no signing keys in the repository, and no signing material in GitHub Secrets.

After cargo-dist uploads the release artifacts, the workflow runs:

```yaml
actions/attest-build-provenance@v1
```

The workflow permissions include:

- `contents: write`
- `id-token: write`
- `attestations: write`

The signer identity is the workflow run itself. For tag `vX.Y.Z`, the CLI updater expects this workflow identity:

```text
https://github.com/ChronoAIProject/NyxID/.github/workflows/release.yml@refs/tags/vX.Y.Z
```

## Self-Update Verification

`nyxid update` does not compile by default. It:

1. Resolves `--version X.Y.Z` to `vX.Y.Z`, or fetches the latest GitHub release.
2. Selects the asset matching the compiled host target.
3. Downloads the archive.
4. Fetches the GitHub Artifact Attestation for the archive's SHA-256 digest.
5. Verifies the Sigstore bundle and exact workflow identity.
6. Extracts `nyxid` or `nyxid.exe`.
7. On Unix, installs the binary into the versioned install root and atomically retargets the active symlink.
8. On Windows, keeps the legacy `self-replace` in-place swap because native symlinks require Developer Mode or elevated privileges.
9. Runs the newly-installed binary with `nyxid ai-setup update` as a bounded child process while retaining the update lock.

If the requested release or compatible prebuilt asset is unavailable, the updater
fails without building source. Attestation failure also fails closed. The
`--insecure-skip-verify` flag has been removed; unattended CLI and machine updates
have no verification bypass or source fallback.

Source builds require an explicit choice:

```bash
nyxid update --from-source --version X.Y.Z
```

The updater resolves that release's tag through GitHub's Git API, peels annotated
tags to a full 40-hex commit, and fetches `cli/Cargo.toml` at that exact commit.
The package version must match the requested release. It then runs
`cargo install --git https://github.com/ChronoAIProject/NyxID --rev <commit> --locked`
into private staging and requires the staged binary to report the exact requested
version before activation. Omitted `--version` selects the latest actual release,
never default-branch HEAD. On Linux arm64, it selects `CC=clang` when available and
reports the existing GCC compiler-guard recovery instructions when applicable.

On Unix, a local source build is recorded separately as
`vX.Y.Z+source.<commit>/nyxid` (appending `.source.<commit>` to an existing build
suffix), with a `.local-source.json` marker. Activation copies into the destination
filesystem, syncs, then atomically replaces the binary entry and active symlink.
It never replaces the trusted unattended controller. Source activation persists
an automatic-update hold first; install an attested prebuilt release and run
`nyxid update auto resume` to resume scheduled upgrades. Enabling or resuming
automatic upgrades while the active binary is a local source build fails clearly.
Windows retains its in-place `self_replace` source-install path; versioned
symlink activation and automatic updates are unsupported there.

### Accepted provenance and authenticated time

Both CLI release archives and machine images use the same verifier. Accepted
bundles contain one in-toto DSSE signature, SLSA v1 provenance and one Rekor v1
`dsse/0.0.1` entry. The certificate must chain to a Fulcio authority trusted at the
signed event time, with code-signing usage, an embedded verified SCT, the GitHub
Actions OIDC issuer, and the exact expected repository/workflow/tag SAN.

The verifier binds the logged payload hash, envelope hash, signature and signing
certificate to the supplied bundle. `envelopeHash` is over the original producer
JSON bytes: the accepted GitHub producer encoding orders fields as `payload`,
`payloadType`, `signatures`, and signature fields as `sig`, `keyid` (including an
empty key id). Unsupported encodings fail closed rather than guessing their
original bytes. The DSSE signature and archive/image SHA-256 subject must verify.

Every accepted Rekor v1 entry requires a valid signed entry timestamp (SET), which
signs the exact body bytes, integrated time, log ID and outer log index. Only
then is `integratedTime` used for certificate/key validity. A body inclusion proof
alone does not authenticate this outer time. Retired CT/Rekor keys remain usable
within their authenticated validity intervals; Fulcio trust intervals and anchor
certificate validity are also checked at event time.

Bundle v0.1 (`application/vnd.dev.sigstore.bundle+json;version=0.1`) permits a SET
without an inclusion proof. v0.2/v0.3 require both SET and inclusion proof. Any
supplied proof is checked, including RFC6962 path/index/size/root and the log's
signed checkpoint. Sharded Rekor outer indexes and shard-local proof indexes can
differ. Proof-only bundles, unsupported versions/kinds, unknown keys, unsigned
time, and malformed or inconsistent supplied evidence are rejected. RFC3161-only
and Rekor v2 evidence are currently unsupported; their presence in the trust root
does not invalidate supported v1 keys. Producer format changes require fixtures
and an explicit reviewed verifier update before release.

API JSON, individual bundle fetches, DSSE payloads, certificates, chain length,
attestation count and Merkle paths have size/count bounds. Downloaded bundle URLs
and each redirect must use an approved GitHub HTTPS origin on port443. The bundle
client does not inherit the caller's default Authorization header. Authenticating
TUF signatures, expiry, version/length/hash checks and the configured TLS roots
remain mandatory. Public producer fixtures and their retrieval hashes are in
`cli/tests/fixtures/sigstore/README.md`.

Native CLI consumers persist TUF rollback state in
`$HOME/.nyxid/tuf-public` (`%USERPROFILE%` on Windows), overridden by
`NYXID_TUF_STATE_DIR`. The backend's updater-image verifier uses the separate
`$HOME/.nyxid/server-tuf` directory by default and accepts an explicit absolute
`NYXID_SERVER_TUF_STATE_DIR`. The backend image sets that variable to
`/var/lib/nyxid/tuf`; Compose retains the directory in the `backend_tuf` volume.
Provision mode0700 owned by the backend UID under trusted parent directories.
Read-only root filesystems require a writable durable mount at that path; a
temporary directory or a new empty volume on every restart loses rollback
protection and is unsupported. Invalid, inaccessible or corrupt state fails
verification without falling back to HOME, ephemeral state or an image tag.
Native servers need no permission to write `/var/lib`. The Docker companion
uses its existing private update-volume `tuf` directory and direct GitHub mirror
transport. Native, backend and companion caches share the same state contract.

On Unix the state directory must belong to the current user with mode0700, and
state/lock files use mode0600 without following symlinks or accepting hardlinks.
Every supplied ancestor and intermediate symlink target is checked while walking
from pinned directory descriptors: owners must be root or the current UID, and
group/other writes require a trusted-owned sticky directory. Platform aliases are
supported only through trusted traversal. Rejection happens before creating state
under an unsafe ancestor. Refreshes serialize through a file lock and publish one fsynced atomic state file containing verified
root transitions, signed timestamp/snapshot metadata and the monotonic clock
floor. Each refresh starts from the latest authenticated root, preserving
rollback floors across restarts and avoiding repeated resets after key rotation.
Malformed, missing previously initialized, or invalidly signed state fails
closed; it is never silently replaced with the bootstrap. State, lock and legacy
metadata opens and atomic publication remain relative to the pinned state inode,
so renaming/replacing its pathname during a refresh cannot redirect those
operations. Linux also anchors tough's disposable scratch workspace through the
directory descriptor and retains that handle for the repository lifetime.
Existing flat tough datastores migrate only after their root lineage and cached roles authenticate.
Do not delete or restore an older trust directory as routine cache cleanup.
These local permissions protect state from other users and accidental corruption;
they cannot detect restoration of the entire filesystem by its administrator.
Place custom state under a trusted home or dedicated service volume whose
existing parent directories cannot be replaced by an untrusted user. First-run
bootstrap remains available in a new empty private directory under trusted
ancestors. Its initialization marker detects subsequent missing state within that
directory. Trusted ancestry prevents another identity from renaming the whole
directory to remove that marker; root or the current UID can still deliberately
replace the whole tree, which cannot be distinguished from a fresh installation
after restart. There is no separate external initialization anchor, and replacing
an entire tree/volume remains unsupported cleanup. New binaries reconcile
embedded bootstrap advancement with the retained authenticated lineage; applicable timestamp/snapshot floors survive
that upgrade. An incompatible embedded root fork is rejected.

## Versioned Install Layout

Unix self-updates use a versioned-directory layout:

```text
~/.local/share/nyxid/versions/
  v0.4.0/nyxid
  v0.4.1/nyxid
  v0.5.0/nyxid
~/.local/bin/nyxid -> ~/.local/share/nyxid/versions/v0.5.0/nyxid
```

The install root is resolved as:

1. `NYXID_INSTALL_ROOT`, when set. This is mainly for tests and controlled installs.
2. `$XDG_DATA_HOME/nyxid/versions`, when `XDG_DATA_HOME` is set.
3. `$HOME/.local/share/nyxid/versions`.

The active symlink defaults to `$HOME/.local/bin/nyxid`. `NYXID_ACTIVE_SYMLINK` can override it. When the currently-running binary is already in a directory on `PATH` and is not inside the versioned install root, the updater prefers that path. This preserves source-built installs at locations such as `~/.cargo/bin/nyxid` until the first prebuilt update migrates them.

The skill's `skills/nyxid/scripts/install.sh` wrapper uses this same layout for
both prebuilt and source installs. It downloads the real cargo-dist 0.30.0
installer into private staging and sets `NYXID_CLI_UNMANAGED_INSTALL=<staging>`
(flat layout, no receipt, no RC modifications) and `NYXID_CLI_NO_MODIFY_PATH=1`.
It clears `NYXID_CLI_INSTALL_DIR` and `CARGO_DIST_FORCE_INSTALL_DIR` for that child:
those controls take precedence over unmanaged staging in 0.30.0. The wrapper
validates the staged `--version`, installs mode 0755, atomically replaces the active
symlink, configures its actual parent directory in the RC file, and verifies the
active executable. Re-runs support symlinks, legacy files and the same version.
Only unsupported targets or installer download/run failure trigger source fallback;
a missing, non-executable, unversioned binary or relocation failure is fatal.
Temporary staging is removed on exit.

Neither the updater nor doctor reads cargo-dist receipts; both use the versioned
root/active path contract above. `scripts/uninstall.sh` removes the self-hosted
Docker deployment and configuration, not the CLI, and does not consume receipts.

CLI HTTPS/WSS trust and daemon CA environment persistence are documented in
[Network, proxies and TLS](site/cli/guides/network.md). All directly constructed
reqwest clients and WSS connectors share `cli/src/tls.rs`. `self_update` is used
only for local archive extraction (`Extract`), not its HTTP/download APIs.
The node IFTTT adapter accepts a builder; the CLI supplies the shared TLS builder
instead of calling the adapter crate's default-client singleton.
Sigstore 0.13's `SigstoreTrustRoot::new` constructs a private tough HTTP client
and exposes no injection hook. The CLI instead builds a separate client with shared TLS roots and tough's
original timeouts/default headers (never GitHub Authorization), retry budget/backoff,
and range recovery. It supplies the client through
`tough::Transport` in `commands/update_attestation/trust.rs`, authenticates the TUF
target, then calls Sigstore's parsing-only constructor. The bootstrap signing
root in `cli/resources/sigstore-root.json` is copied byte-for-byte from
sigstore 0.13.0's `trust_root/prod/root.json` (upstream sigstore-rs, Apache-2.0).
When upgrading Sigstore, review this bootstrap root alongside that dependency;
normal TUF root rotation remains signature-verified. TLS CA configuration never
bypasses TUF/attestation verification.

On every successful Unix prebuilt update, the updater:

- Writes the new binary to `{install-root}/{tag}/nyxid` with mode `0755`.
- Creates a temporary symlink in the active binary's directory and renames it over the active path, so retargeting is atomic on POSIX systems.
- Keeps the active version plus the two most recent previous version directories,
  plus additional versions pinned by installed node service definitions.

Archive extraction is staged before publication and always publishes the flat
`{tag}/nyxid` layout. Retention and rollback also recognize older nested
`{tag}/nyxid-cli-{target}/nyxid` installations. Reinstalling an active tag replaces
the directory entry atomically instead of truncating the running executable.

Opt-in automatic upgrades use the same verifier, activation and retention paths.
Enabling the scheduler validates explicit TLS configuration and persists only
`NYXID_CA_CERT`, `SSL_CERT_FILE` and `SSL_CERT_DIR` as absolute, escaped paths in
launchd/systemd, using the same helper as node daemon installation. Proxy variables
are never persisted. Re-run `nyxid update auto enable` with the intended interval
to update the CA paths; omitted or empty variables remove previous entries.
Native OS roots are lazy fallback roots, loaded only after eager verification
fails (or explicitly by doctor); scheduled updates use the same full trust union.
See [automatic upgrade operations](site/cli/getting-started/install.md#automatic-upgrades)
for scheduling, holds, controller recovery, skills, and deferred node adoption.

Legacy single-file installs migrate automatically. If the active path is a regular file, the first successful prebuilt update writes the versioned binary and replaces that regular file with the symlink. The updater does not try to reconstruct old versions that it did not install itself.

Useful maintenance commands:

```bash
nyxid update --list-versions
nyxid update --rollback
```

`--list-versions` prints the installed version directories and marks the active version. `--rollback` retargets the active Unix symlink to the previous retained version; it errors if there is no previous version. Windows users continue to get the in-place `self-replace` updater and do not have rollback support.

## Installer URLs

Unix users should install through the repository wrapper:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
```

The wrapper delegates to cargo-dist's prebuilt shell installer, reads the installed `nyxid --version`, moves the resulting regular file into the versioned layout, and replaces the active path with a symlink. This is the chosen approach instead of cargo-dist install-path templating so the generated installer remains stock and the versioned-layout policy stays in NyxID-owned script code.

The generated cargo-dist shell installer remains attached to each release:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/ChronoAIProject/NyxID/releases/latest/download/nyxid-cli-installer.sh | sh
```

Running that generated installer directly writes a regular binary into `~/.local/bin`; the next `nyxid update` migrates it to the versioned layout.

Windows is not yet supported as a release target — see "What CI Builds" above. Run `nyxid` under WSL.

The repository skill installer at `skills/nyxid/scripts/install.sh` wraps the shell installer and falls back to `cargo install` only when a usable prebuilt binary is not available for the host.

## Doctor

`nyxid doctor` prints a local health check covering:

- Installation path, active symlink, `PATH`, and installed version.
- GitHub Releases reachability, latest release, and rate limit headers.
- Stored base URL and login state with tokens redacted.
- Telemetry consent state.
- Startup update-check cache state.

`nyxid doctor --json` emits the same data as structured JSON for scripts. The command exits with status `1` only when a row is a hard failure; warnings such as "not logged in" do not fail the command.

## Startup Update Notice

For normal interactive commands, `nyxid` starts a telemetry-free update availability check in the background and prints a short stderr notice only when a newer release is available.

The check:

- Calls only `https://api.github.com/repos/ChronoAIProject/NyxID/releases/latest`.
- Is skipped for `nyxid update`, `CI=true`, non-TTY stdout, or `NYXID_NO_UPDATE_CHECK=1`.
- Uses a 1500 ms ceiling and never delays command output waiting for GitHub.
- Caches successful and failed attempts at `~/.nyxid/update-check.json` for 24 hours.

The cache format is:

```json
{"last_checked":"2026-05-07T08:00:00Z","latest_known":"v0.5.0"}
```

`nyxid doctor` reads this cache in its "Update check" section.

## If Release CI Fails

First determine where it failed:

- Build job failed before publishing: fix `main`, then either rerun the workflow if the failure was transient or cut a new patch tag after the fix.
- Host/upload job failed before a GitHub release exists: rerun the workflow for transient infrastructure failures; otherwise fix `main` and cut a new patch tag.
- A GitHub release or artifacts were partially published: remove the broken release from GitHub, leave the failed workflow logs intact for auditability, fix `main`, and cut a new patch tag. Do not replace signed artifacts in-place under the same published tag.
- Attestation failed: inspect the `Attest release artifacts` step and workflow permissions. Do not publish binaries without attestations.

Before retrying a release, validate locally:

```bash
dist plan --output-format=json --tag=vX.Y.Z
cargo build -p nyxid-cli
cargo test -p nyxid-cli
cargo clippy -p nyxid-cli --all-targets -- -D warnings
cargo fmt --all -- --check
```
