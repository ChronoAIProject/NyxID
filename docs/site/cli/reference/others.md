---
title: Other commands
description: Reference for the remaining nyxid commands — updating the CLI, account & MFA, sessions, approvals, notifications, endpoints, credentials, service accounts, developer apps, channel relay, admin, and telemetry.
---

The pages above cover the headline command groups. This is the catch-all reference for everything else `nyxid` exposes: keeping the CLI itself current, account and session management, approvals and notifications, endpoints and credentials, service accounts and developer apps, channel relay, admin, and telemetry. Run `nyxid --help` or `nyxid <command> --help` for the authoritative flag list.

:::note
Commands that call the NyxID API accept the common flags `--base-url`, `--access-token` / `--access-token-env`, `--profile`, and `--output table|json`. Account-bootstrap commands (`login`, `register`, password reset) take `--base-url` explicitly. See [Authenticate](/docs/cli/getting-started/authenticate).
:::

:::tip
Secret-issuing commands (`api-key create`, `service-account create`, `node register-token`, `*-rotate`, …) open a browser wizard so the new secret is shown in a page you control. Add `--terminal` to print it to the terminal instead, or `--no-wait` to create a remote pairing and pick the result up later with [`nyxid pairing resume`](#pairing).
:::

## update

Keep the CLI and your installed AI skills current. With no flags, `nyxid update` upgrades the `nyxid` binary to the latest GitHub release, verifies the release attestation, then hands off to the freshly-installed binary to refresh installed skills.

```bash
nyxid update                     # update the CLI binary, then skills
nyxid update --check             # print installed vs. latest, install nothing
nyxid update --version 0.4.0     # pin a specific release
nyxid update --skills-only       # refresh AI skills only, skip the binary
nyxid update --rollback          # point nyxid back at the previously retained version
nyxid update --list-versions     # list installed prebuilt versions
nyxid update --from-source       # compile + install via cargo (unsupported targets)
```

- The updater retains the **3** most recent prebuilt versions so `--rollback` can revert a bad upgrade instantly.
- `--from-source` runs `cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli --force --locked`; use it on targets without a compatible prebuilt binary, or as a contributor.
- On Linux arm64 source builds, the updater uses `CC=clang` when `clang` is available and otherwise reports the `CC=clang` workaround if it detects the `aws-lc-sys` `gcc#95189` compiler guard.
- `--insecure-skip-verify` continues even if attestation verification fails — avoid it unless you understand the risk.
- `--base-url` selects the NyxID instance to fetch skill content from (defaults to your saved login URL).

## ai-setup

Install and maintain the persistent NyxID skill files that teach a coding assistant to broker credentials through the CLI. See [Set up Claude Code, Cursor & Codex](/docs/ai/guides/claude-code-cursor-codex).

```bash
nyxid ai-setup install --tool claude-code   # claude-code | cursor | codex | openclaw | generic
nyxid ai-setup update [--tool <tool>]        # update all installed skills, or just one
nyxid ai-setup status                        # show which tools have skills installed
```

## Account & sign-in

Bootstrap and tear down the locally stored session. Day-to-day login lives in [Authenticate](/docs/cli/getting-started/authenticate).

```bash
nyxid login --base-url <BASE_URL>        # browser sign-in (add --password for email/password)
nyxid logout                             # clear the stored session
nyxid register --base-url <URL> --email <addr> --invite-code <CODE>
nyxid verify-email --token <TOKEN>       # confirm a new account
nyxid forgot-password --email <addr>     # request a reset email
nyxid reset-password --token <TOKEN>     # set a new password (use --password-env)
nyxid whoami                             # current identity
nyxid status                             # session + instance summary
nyxid doctor [--json]                    # local connectivity / config diagnostics
```

## profile

Manage your user profile and OAuth consents.

```bash
nyxid profile update --name "New Name"
nyxid profile delete [--yes]             # delete your account
nyxid profile consents                   # list apps you've granted access
nyxid profile revoke-consent <client_id> [--yes]
```

## mfa

Manage time-based one-time-password (TOTP) multi-factor auth.

```bash
nyxid mfa setup            # browser wizard: QR code, verification, recovery codes
nyxid mfa setup --terminal # scripted path: prints secret + provisioning URL
nyxid mfa verify --code <CODE>
nyxid mfa status
```

## session

```bash
nyxid session list         # active sessions for your account
```

## notification

Control how approval requests reach you.

```bash
nyxid notification settings
nyxid notification update --approval-email true --approval-push false --approval-telegram true
nyxid notification telegram-link        # link a Telegram account
nyxid notification telegram-disconnect
```

## approval

Review and decide credential-use approvals, manage standing grants, and set per-service policy. For the model see [Approvals](/docs/shared/concepts/approvals). Add `--org <id|slug|name>` to act on an organization you administer.

```bash
nyxid approval list                      # pending + recent requests
nyxid approval show <id>
nyxid approval approve <id>
nyxid approval deny <id> [--reason <text>]
nyxid approval grants                    # standing time-based grants
nyxid approval revoke-grant <id> [--yes]
nyxid approval enable                    # require approval globally
nyxid approval disable [--yes]
nyxid approval service-configs           # per-service policy list
nyxid approval set-config <id> --require-approval true --approval-mode grant
```

`set-config` takes a `UserService` ID (from [`nyxid service list`](/docs/cli/reference/service)) or a catalog ID; `--approval-mode` is `per_request` (every call) or `grant` (approval mints a reusable grant).

## endpoint

Manage the raw target URLs (`UserEndpoint`) behind your services. See [Endpoints, keys & services](/docs/shared/concepts/endpoints-keys-services).

```bash
nyxid endpoint list
nyxid endpoint update <id> --url <new-url>
nyxid endpoint delete <id> [--yes]
```

## external-key

Manage the external credentials (`UserApiKey`) your services inject — API keys, bearer tokens, OAuth tokens.

```bash
nyxid external-key list
nyxid external-key rotate <id> --credential-env <ENV_VAR>
nyxid external-key delete <id> [--yes]
```

## provider

```bash
nyxid provider disconnect <provider_id> [--org <id|slug|name>]
```

Disconnect a connected provider token. `--org` targets an org-owned token (admin required).

## oauth bindings

Inspect and revoke broker bindings — server-side credential handles issued for delegated access. See [OAuth & OIDC identity](/docs/shared/concepts/oauth-oidc).

```bash
nyxid oauth bindings list
nyxid oauth bindings show <hash>         # full SHA-256 hex, or an 8+ char prefix
nyxid oauth bindings revoke <hash> [--yes]
```

## service-account

Machine-to-machine identities for the OAuth2 `client_credentials` grant. The `client_secret` is shown once at create / rotate.

```bash
nyxid service-account create --name <name> --scopes "openid profile" [--org <id|slug|name>]
nyxid service-account list [--org <id|slug|name>] [--search <q>]
nyxid service-account show <id>
nyxid service-account update <id> [--name|--scopes|--is-active <bool>]
nyxid service-account delete <id> [--yes]
nyxid service-account rotate-secret <id>     # revokes existing tokens
nyxid service-account revoke-tokens <id>     # revoke all active tokens, keep the SA
```

## developer-app

Register OIDC clients so downstream apps can "Sign in with NyxID." See [Register a developer app](/docs/web/guides/developer-apps).

```bash
nyxid developer-app create --name <name> --redirect-uri <uri> [--client-type public|confidential]
nyxid developer-app list [--org <id|slug|name>]
nyxid developer-app show <id>
nyxid developer-app update <id> [--name|--redirect-uri|--allowed-scopes|--delegation-scopes]
nyxid developer-app delete <id> [--yes]
nyxid developer-app rotate-secret <id>       # confidential clients only
```

## node-credential

Push credential-setup metadata to a [credential node](/docs/cli/reference/node) operator, who then accepts it locally with `nyxid node credentials accept`. See [Set up a credential node](/docs/cli/guides/credential-node).

```bash
nyxid node-credential push <node> --slug <service> \
  --injection-method header|query-param|path-prefix \
  --field-name <name> [--target-url <url>] [--label <text>]
nyxid node-credential list <node>            # pending pushes
nyxid node-credential cancel <node> <pending_id> [--yes]
```

## channel-bot

Bridge a messaging platform (Telegram, Discord, Lark/Feishu, Slack) to an agent API key. See [Connect a channel bot](/docs/cli/guides/channel-bots).

```bash
nyxid channel-bot register --platform telegram --label support --token-env <ENV>
nyxid channel-bot update <id> --verification-token <tok> [--encrypt-key <key>]
nyxid channel-bot list [--org <id|slug|name>]
nyxid channel-bot show <id>
nyxid channel-bot delete <id> [--yes]
nyxid channel-bot verify <id>                # re-check token + re-register webhook
```

### channel-bot route

Map platform conversations to the agent that should answer them.

```bash
nyxid channel-bot route create --bot-id <id> --agent-key-id <id> [--conversation-id <id>] [--default-agent]
nyxid channel-bot route list [--bot-id <id>]
nyxid channel-bot route update <id> [--agent-key-id|--default-agent|--active <bool>]
nyxid channel-bot route delete <id> [--yes]
```

## channel-event

Push device / analyzer events through the HTTP Event Gateway to an agent. Requires an agent API key (`nyxid_ag_…`) bound to the target conversation — session tokens are rejected.

```bash
nyxid channel-event push --conversation-id <id> --source <name> --type <event> \
  [--payload-json '{...}' | --payload-file <path>] --api-key-env <ENV>
```

### channel-event channel

Device channels are conversation rows with no backing bot — somewhere for the gateway to address events.

```bash
nyxid channel-event channel create --conversation-id <name> --agent-key-id <id>
nyxid channel-event channel list [--org <id|slug|name>]
nyxid channel-event channel delete <id> [--yes]
```

## openclaw

One-shot interactive setup that connects a self-hosted OpenClaw gateway as a NyxID service.

```bash
nyxid openclaw setup [--url <gateway-url>] [--token-env <ENV>]
```

(For the node-agent side — running OpenClaw behind a credential node — use `nyxid node openclaw`.)

## telemetry

Edit the persisted anonymous-telemetry consent flag on this machine.

```bash
nyxid telemetry status     # resolved state + its source
nyxid telemetry enable     # opt in
nyxid telemetry disable    # opt out and clear the local anon ID
```

## pairing

Resume a `--no-wait` remote pairing started by a secret-issuing command and print its result.

```bash
nyxid pairing resume <PAIRING_ID>
```

## admin

Platform administration — requires the `admin` role.

```bash
nyxid admin invite-code create [--max-uses <n>] [--note <text>]
nyxid admin invite-code list
nyxid admin invite-code deactivate <id>
nyxid admin user list [--search <q>] [--page <n>] [--per-page <n>]
nyxid admin user show <id>
nyxid admin user set-role <id> --role admin|operator|user
```

## info & repo

```bash
nyxid info                 # CLI version + project links
nyxid repo [--open]        # print (or open) the project repository URL
```
