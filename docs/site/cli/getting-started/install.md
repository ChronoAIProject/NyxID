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
