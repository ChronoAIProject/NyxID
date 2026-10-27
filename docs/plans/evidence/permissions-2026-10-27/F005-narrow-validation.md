# Permissions F005 narrow validation

Source commit: `7185b5ef7eb56194b2e1cbf42f045536a9cb7847`; HEAD: `dd83cc01fbb8bf1f6a70bab6c330f765ade90d59` (docs-only above source).
Binary SHA-256: `1fbad9ed97638d877114c88593fb9db5e615eb27cbda66c9cd0eb613cf37d875`.
11-file manifest SHA-256: `d17bfd6a8c9fa611aec069828005328851c68c14842c731112c39bf8b4478c6c`.

| Check | Exit | Elapsed s | Free GiB start / min / end | Log |
| --- | --- | --- | --- | --- |
| mongo-4 | 0 | 0.5 | 15.607 / 15.607 / 15.607 | `/tmp/nyxid-permissions-r3-F005-narrow-mongo-4.log` |
| compile | 0 | 633.15 | 15.607 / 11.702 / 14.55 | `/tmp/nyxid-permissions-r3-F005-narrow-compile.log` |
| classifier-inventory | 0 | 7.65 | 14.55 / 14.549 / 14.549 | `/tmp/nyxid-permissions-r3-F005-narrow-classifier-inventory.log` |
| classifier | 0 | 0.1 | 14.549 / 14.549 / 14.549 | `/tmp/nyxid-permissions-r3-F005-narrow-classifier.log` |
| permission-inventory | 0 | 0.09 | 14.55 / 14.549 / 14.549 | `/tmp/nyxid-permissions-r3-F005-narrow-permission-inventory.log` |
| permission | 0 | 4.59 | 14.549 / 14.549 / 15.549 | `/tmp/nyxid-permissions-r3-F005-narrow-permission.log` |
| fmt | 0 | 13.65 | 15.549 / 15.549 / 15.549 | `/tmp/nyxid-permissions-r3-F005-narrow-fmt.log` |
| clippy | 0 | 353.86 | 15.549 / 14.514 / 15.525 | `/tmp/nyxid-permissions-r3-F005-narrow-clippy.log` |

Both exact tests had one inventoried entry and passed 1/0/0 (passed/failed/ignored). No broad Codex, historical169, frontend or live acceptance run.
All owned groups exited with no survivors; final free disk 15.524 GiB. All commands inherited nofile65536; shared target/jobs2/config preserved; no resource stop or source/cache/profile/lock changes.

Actual Clippy backend Cargo argv:
```json
["/Users/chronoai/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo", "check", "--config", "/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml", "--manifest-path", "backend/Cargo.toml", "--all-targets"]
```

Combined I05/I06, live acceptance and issue verdicts remain OPEN/BLOCKED; no implementation approval claimed.
