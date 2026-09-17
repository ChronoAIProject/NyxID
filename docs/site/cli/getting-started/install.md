---
title: Install the CLI
description: Install the nyxid command-line tool on macOS, Linux, or Windows (WSL) and verify it works.
---

The `nyxid` CLI covers every user-facing NyxID operation — services, keys, catalog, nodes, approvals, SSH, MCP, and notifications — plus the `nyxid node` subcommand for running on-premise credential nodes. It is the fastest way to script NyxID and the only surface some workflows need.

## Install (macOS & Linux)

Run the install script, then make sure the binary is on your `PATH`:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
export PATH="$HOME/.local/bin:$PATH"
```

The installer uses attested prebuilt binaries for macOS x64/arm64 and Linux x64/arm64. Linux arm64 binaries target Ubuntu 20.04 / `glibc 2.31`, so Jetson-class Ubuntu 20.04 hosts use the prebuilt path instead of compiling locally.

## Verify

```bash
nyxid --help
```

You should see the top-level command list. If `nyxid` is not found, re-run the `PATH` line above (or open a new terminal).

:::warning
**Windows:** run `nyxid` from a Unix-compatible shell — WSL Ubuntu (recommended) or Git Bash. The raw Windows command prompt is not supported.
:::

## Update

Keep the CLI current with its built-in updater — it upgrades the binary to the latest release, then refreshes any installed AI skills:

```bash
nyxid update           # update the CLI, then skills
nyxid update --check   # see installed vs. latest without installing
```

See [Other commands → update](/docs/cli/reference/others#update) for version pinning, rollback, and source builds.

### Automatic upgrades

Versioned prebuilt installations on macOS and Linux can opt into verified,
scheduled upgrades:

```bash
nyxid update auto enable --interval-hours 24
nyxid update auto status --json
nyxid update auto hold
nyxid update auto resume
nyxid update auto disable
```

The macOS LaunchAgent or Linux systemd user timer checks eligibility every minute,
without a terminal. The interval is 1 to 720 hours. Sleeping hosts and inactive
user schedulers defer checks; Linux hosts that must run after logout need an
administrator to enable user lingering. Status reports whether the scheduler is
loaded, the next eligible check, version hold, last attempt and result.
`nyxid doctor` also reports the local policy. Ordinary CLI commands do not run
automatic installations.

Automatic upgrades accept stable prebuilt releases with valid GitHub artifact
attestations. They never compile from source or skip verification. Manual updates,
automatic runs, and rollback share one installation lock. Interrupted downloads
or invalid attestations leave the previous binary usable. Controller refresh,
retention cleanup and skills refresh retry if activation succeeded but later work
failed. To request the next due run immediately, use `auto resume` followed by
`auto run` while the policy is enabled.

`nyxid update --rollback` also creates a version hold. It remains held until you
explicitly resume, and a failed rollback with no candidate creates no hold. A
retained `.update-controller` executable manages the scheduler even after rollback
to a release that predates these commands. Use the exact controller path printed
by `auto enable`, for example:

```bash
"$HOME/.local/share/nyxid/versions/.update-controller" update auto status --json
"$HOME/.local/share/nyxid/versions/.update-controller" update auto disable
```

The controller detects its installation root, including custom roots. `disable`
and `enable` recover a malformed policy file; `disable` saves the disabled policy
before removing the scheduler. For controller recovery with a malformed policy
and custom paths, pass `NYXID_INSTALL_ROOT` and `NYXID_ACTIVE_SYMLINK` explicitly.

Skills are refreshed through the installed binary's `ai-setup update` command.
Existing `nyxid node` processes keep their running binary and active proxy/SSH
sessions. Automatic upgrades always defer node adoption. Restart nodes during an
operator-selected maintenance window. Older daemon installations may pin a
versioned executable; migrate each such service once with
`nyxid node daemon install --force --profile NAME` during that maintenance window.
On macOS, `--force` stops the existing process immediately before rewriting its
definition. Pass the original `--config /absolute/config-directory` and
`--log-level LEVEL` when customized; an omitted config reuses saved daemon metadata,
but an omitted log level uses the default. Then start the service with
`nyxid node daemon start --profile NAME`. The new definition uses the stable
installation symlink. The updater retains
versions pinned by existing service definitions in addition to the normal three
retained versions. Unsupported service-definition syntax stops retention cleanup
and is reported; inspect and reinstall that service before retrying.

## Build from source (contributors)

If you are working on NyxID itself rather than just using it, build the CLI from the repository:

```bash
cargo install --path cli
nyxid --help
```

On Linux arm64, install `clang` first or set `CC=clang` for source builds. The CLI dependency graph includes `aws-lc-sys`, which can reject affected GCC versions with the `gcc#95189` compiler guard.

## Next

- [Authenticate](/docs/cli/getting-started/authenticate) — log in and point the CLI at your NyxID instance.
- [Your first connection](/docs/cli/getting-started/first-connection) — connect a service and make a proxied call.
