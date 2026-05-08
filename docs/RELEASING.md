# Releasing NyxID

This document covers the release path for the `nyxid` CLI binaries produced by CI.

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
- `aarch64-unknown-linux-gnu` on `ubuntu-24.04-arm`
- `x86_64-apple-darwin` on `macos-latest`
- `aarch64-apple-darwin` on `macos-latest`
- `x86_64-pc-windows-msvc` on `windows-latest`

Linux release runners install `libdbus-1-dev` and `pkg-config` because the CLI uses the keyring crate's Secret Service backend on Linux. The wizard bundle is already committed under `cli/src/wizard/assets/`, so release builds do not need Node.

cargo-dist publishes:

- Platform archives: `nyxid-cli-<target>.tar.gz` for Unix and `nyxid-cli-<target>.zip` for Windows
- SHA-256 checksums
- `nyxid-cli-installer.sh`
- `nyxid-cli-installer.ps1`
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
9. Re-execs the newly-installed versioned binary with `nyxid update --skills-only` so the new binary owns skill updates.

If no prebuilt asset exists for the host target, the updater clearly falls back to:

```bash
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli --force --locked
```

Verification failures never fall back to source automatically. They fail closed unless the user explicitly passes `--insecure-skip-verify`.

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

On every successful Unix prebuilt update, the updater:

- Writes the new binary to `{install-root}/{tag}/nyxid` with mode `0755`.
- Creates a temporary symlink in the active binary's directory and renames it over the active path, so retargeting is atomic on POSIX systems.
- Keeps the active version plus the two most recent previous version directories, for three retained versions total.

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

Windows users can use the PowerShell installer from the same release:

```powershell
irm https://github.com/ChronoAIProject/NyxID/releases/latest/download/nyxid-cli-installer.ps1 | iex
```

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
