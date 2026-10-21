# Reviewed Node dependency installation

The updated frontend, SDK, offline validator, updater transport and Node-tool
roots intentionally disable dependency lifecycle hooks with their committed
`.npmrc` (`ignore-scripts=true`). Their `package.json` files also record explicit
`allowScripts: false` decisions. `security/node-install-policy.json` binds each
review to its exact lock path, package name, version and archive integrity.
An empty review list means the root currently has no install-script dependency.

The current denied hooks are:

| Package | Decision and supported execution path |
| --- | --- |
| core-js | Its optional funding/notification hook produces no required runtime code. |
| fsevents | Release builds need no watcher compilation. Installed integrity-locked prebuilt optional files remain available where supported. |
| esbuild | Deny installer download/repair fallback. The SDK uses its integrity-locked platform optional package. Actual SDK builds/tests validate this path. |
| @anthropic-ai/claude-code | Deny the native installer/downloader. The repository offline-validation launcher uses the integrity-locked native optional package and permits only schema validation. |

This policy does not approve lifecycle execution for future package versions.
The CI guard rejects a new script-bearing lock entry, changed version/integrity,
missing/stale review, wildcard approval, missing root, or config that enables
scripts. Review a dependency update's installed-file requirements and record its
exact new identity before changing the policy. Do not use a general script
approval, force, or legacy peer resolver to make an install pass.

Use the reviewed npm11.21.1 bootstrap from `security/build-tools.json`. The
packageManager field records the contract but does not select the executable.
Docker builders copy the committed `.npmrc` before `npm ci`; CI uses the same
reviewed npm. Bootstrapping the integrity-locked Node tool root itself disables
scripts even when the Node base image supplies the initial npm10 executable.
The wizard source freshness inputs include its install policy.

`npm run build` and other explicitly requested project commands still work with
`ignore-scripts=true`. The setting suppresses dependency lifecycle hooks, not
the named project command. The SDK uses the supported `--workspaces` flag.

```sh
python3 scripts/check-node-install-policy.py
python3 scripts/test_node_install_policy.py
```

Fresh installs and actual product/schema-validation checks provide execution
evidence beyond the static guard. Other roots join this catalog when their
dependency remediation is complete. The interrupted mobile patch verification
remains open and is not covered by this acceptance evidence.
