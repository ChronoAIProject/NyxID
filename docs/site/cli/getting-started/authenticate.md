---
title: Authenticate
description: Log the nyxid CLI into your NyxID instance, check your session, and manage multiple accounts with profiles.
---

The CLI authenticates once and reuses a locally stored session for every subsequent command. You only repeat this when the session expires or when you switch instances.

## Log in

```bash
nyxid login --base-url <BASE_URL>
```

`nyxid login` starts device-code approval and stores the selected identity under `~/.nyxid/`. The approver chooses a full account session or a restricted Agent Key. Use the API base URL for `<BASE_URL>`:

- **Hosted:** `https://nyx-api.chrono-ai.fun`
- **Self-host:** `http://localhost:3001` (the API runs on 3001; the web console is on 3000)

### The default and browser-callback alternative

Plain `nyxid login` deliberately defaults to selectable **device-code v2**, on desktop too. It is the only terminal-originated flow where the approving human sees the requester attribution panel (IP, timezone, origin, screen) and chooses between a full account session and a restricted Agent Key (#1535). This is why 0.16.0 (`78cb26b`) changed the default; that commit message omitted the rationale. The legacy local-callback flow always grants a full account session without a requester review step.

The CLI prints the code and opens only the bare `verification_uri`, never `verification_uri_complete`. Manual entry keeps the deliberate code-transfer step used by GitHub CLI. To paste instead of retyping, opt into `-c` / `--clipboard`:

```bash
nyxid login --clipboard --base-url <BASE_URL>
nyxid login --callback --base-url <BASE_URL>
```

`--clipboard` copies only the user code before browser opening, using the system clipboard tool. Copy failures produce one stderr message and login continues normally. With `--output json` or `--no-wait`, the CLI prints the code without copying it or opening a browser; JSON mode still polls unless `--no-wait` is also set.

`--callback` uses the local browser-callback login: it opens the web console and completes with a full account session, with no code entry or requester review. If the browser cannot open, it falls back to device-code login. It conflicts with `--password`, `--device`, `--agent-key`, `--code`, and `--no-wait`. `NYXID_LOGIN_NO_DEVICE_FALLBACK=1` remains the legacy equivalent of `--callback` for plain blocking login. Explicit device, Agent Key, one-time-code, resume, and structured JSON modes keep their existing exchange behavior.

### Headless / SSH / no browser

Select device-code login explicitly to approve from a phone or another computer:

```bash
nyxid login --device --base-url <BASE_URL>
```

Open the printed bare URL, enter the code, press **Continue**, and review the requesting device before choosing access and approving or rejecting. Explicit device/Agent Key login prompts to open a browser when stdin and stderr are TTYs; without a TTY it prints the challenge and polls. Plain login in CI requires an explicit mode such as `--device --no-wait`; unattended jobs should normally use a pre-issued Agent Key.

The approval page's phone QR/deep link prefills both the web page (including an ordinary camera scan) and the mobile app. The web page formats the code once and immediately removes it from the URL; malformed codes leave an empty input with an explanation. Prefill makes no request or decision. Press **Continue**, confirm the echoed code matches the requesting device or terminal, and reject a mismatch. Approval always requires review and an explicit decision.

### Agent-driven login and resume

```bash
nyxid login --no-wait --profile work --base-url <BASE_URL> --output json
nyxid login resume <REQUEST_ID> --profile work --output json
nyxid login resume <REQUEST_ID> --profile work --once --output json
```

The first command creates one request and returns its human code, bare verification
URL, expiry, poll interval and nonsecret local resume handle. It opens no browser
and never waits for approval. The poll secret stays in a mode-0600 local pending
file. A handle works only on the same machine/profile/destination; it cannot
redeem a login remotely. `--once` performs at most one eligible poll and preserves
the server's backoff and next poll deadline.

Resume returns one JSON result. Authentication reports `auth_kind` as
`account_session` or `agent_key`; no credential is printed. Stable errors have
the shape `{"error":{"code":"login_pending","message":"..."}}`:

| Exit | Code |
|---|---|
| 10 | `login_pending` |
| 11 | `login_denied` |
| 12 | `login_expired` |
| 13 | `login_already_delivered` |
| 14 | `login_rate_limited` |
| 15 | `login_resume_busy` |
| 16 | `login_request_not_found` |
| 17 | `login_destination_mismatch` |
| 18 | `login_code_invalid` |
| 19 | `login_unavailable` |
| 20 | `login_unsupported` |
| 21 | `login_storage_failed` |

### Login with a one-time code

Open **Settings > Create login code** on the web, or the equivalent account-settings
action in the mobile app. Choose account access or select/create a restricted key
and confirm its permissions. Enter the displayed five-minute code on the intended
machine:

```bash
nyxid login --code --profile work --base-url <BASE_URL>
```

The command prompts for the code without displaying it. The optional argument
form `--code XXXX-XXXX` supports automation but may enter shell history.
Codes are consumed once. The issuing screen shows redemption and requester
context; Cancel stops a pending code, while Revoke invalidates its delivered
session or child credential. Neither the code nor a poll secret belongs in URLs
or browser storage. A browser-owned QR login approved as restricted returns a
one-time CLI handoff code instead of an account cookie.
Creating that handoff extends the initial 60-second delivery window to five
minutes after approval. Repeated handoff requests never extend the fixed deadline.

### Non-interactive credentials

Authenticated commands resolve credentials in this order: `--access-token`, the variable named by `--access-token-env` (default `NYXID_ACCESS_TOKEN`), `NYXID_API_KEY` as a fallback alias for the default selector, then the stored profile session. If both default variables are set, `NYXID_ACCESS_TOKEN` takes precedence. A custom `--access-token-env <VAR>` uses only `<VAR>` before falling back to the stored session.

Caller-selected credentials are fail-closed: if the server rejects one, the CLI exits nonzero and does not refresh or retry with the stored session identity.

## Agent Key login

Authorize a CLI profile with a restricted Agent Key instead of an account session:

```bash
nyxid login --agent-key --profile home-agent --base-url <BASE_URL>
```

The CLI prints a one-time code and the bare `/login/agent-key` verification URL. Enter the code and choose **Approve on this computer** to sign in and review the request, or **Approve from your phone** to display a QR code. Scan it with the NyxID mobile app or an ordinary camera app to prefill the mobile or web approval page. Press Continue, match the echoed code against the requesting device or terminal, then explicitly approve or reject. The phone path creates no account login in the requesting computer's browser.

Review the requesting device, profile, IP attribution, location, and time. Choose an existing eligible personal key or a key owned by an organization you administer, or create a new key. New keys default to `read proxy`, no allowed services or nodes, and a 90-day expiry; select the resources and expiry you intend to grant. The final confirmation shows effective permissions, resource names, allow-all warnings, expiry, and rate limits. You can reject instead of approving.

Approval issues a new login credential bound to the selected key. Selecting an existing key does not change its secret, scopes, or other consumers. The CLI receives the credential once, directly from NyxID. It stores it in the profile's `token` file with mode `0600`, plus an `auth_kind` marker, safe identity metadata in `agent_key.json`, and the backend URL. Stale account access tokens, refresh tokens, and user IDs are removed. No account session is stored, and unsupported backends fail without falling back to account login.

`nyxid whoami` and `nyxid status` identify **Authentication: Agent Key**. The key's live scopes, service and node restrictions, bindings, rate limits, and expiry remain authoritative. Credentials cannot outlive a key's expiry. Rejected credentials fail without refreshing or switching identities; `nyxid session refresh` exits with code 3 because Agent Key sessions do not refresh.

Identity output includes the credential's hostname/profile label. `status` then lists the account, AI services, API keys, and nodes; sections denied by the key's scope display "unavailable with this key's scope". JSON output includes an `auth` object and uses `null` for unavailable sections. A missing local credential prompts reauthorization for that profile; a server rejection still fails the command.

`nyxid logout --profile home-agent` attempts bounded server revocation and clears the matching local login, reporting whether revocation succeeded. A newer concurrent login is preserved. In the web console, open the key's **Login credentials** section to revoke a specific CLI login. Revoking or rotating the key invalidates every credential issued under it. Revocation and expiry take effect on subsequent authenticated requests. Abandoned approvals expire after a 60-second delivery window and their credentials are revoked automatically; a browser-owned device handoff uses the fixed five-minute deadline described above. A parent created for an abandoned request remains as key configuration with no disclosed primary secret; cleanup revokes only that request's child so another approved consumer remains usable.

`--agent-key` cannot be combined with `--device`, `--password`, `--callback`, or `--code`. It supports headless polling, including a human authorizing a waiting CI job; unattended jobs should normally use a pre-issued credential through the existing environment-variable options.

## Check your session

```bash
nyxid whoami     # who you're logged in as
nyxid status     # session + instance summary
nyxid doctor     # diagnose connectivity / config problems
```

## Profiles — multiple accounts or instances

Every command accepts `--profile <name>` (or the `NYXID_PROFILE` environment variable) to keep separate sessions side by side — for example a personal account and an org account, or hosted vs. local.

```bash
nyxid login --profile work --base-url https://nyx-api.chrono-ai.fun
nyxid --profile work whoami
```

Profile sessions live under `~/.nyxid/profiles/<name>/`; the default profile uses `~/.nyxid/` directly. Profile names allow letters, numbers, hyphens, and underscores (1–64 chars).

:::tip
Profiles also scope the `nyxid node` daemon, so you can run multiple credential-node instances on one machine without them colliding.
:::

## Next

- [Your first connection](/docs/cli/getting-started/first-connection) — connect a service and verify a proxied call.
