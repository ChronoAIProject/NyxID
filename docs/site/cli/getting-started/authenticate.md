---
title: Authenticate
description: Log the nyxid CLI into your NyxID instance, check your session, and manage multiple accounts with profiles.
---

The CLI authenticates once and reuses a locally stored session for every subsequent command. You only repeat this when the session expires or when you switch instances.

## Log in

```bash
nyxid login --base-url <BASE_URL>
```

`nyxid login` opens your browser, completes sign-in, and stores the session under `~/.nyxid/`. Use the API base URL for `<BASE_URL>`:

- **Hosted:** `https://nyx-api.chrono-ai.fun`
- **Self-host:** `http://localhost:3001` (the API runs on 3001; the web console is on 3000)

### Headless / SSH / no browser

If `nyxid login` can't open a browser (SSH session, container, WSL without `$DISPLAY`), it auto-falls back to the **device-code flow**: the CLI prints a one-time code + a URL, you open the URL on any signed-in browser (phone, laptop), type the code, review the requester IP and time, then approve or reject. Approval completes the CLI login; rejection stops it immediately. You can also force the flow explicitly:

```bash
nyxid login --device --base-url <BASE_URL>
```

Set `NYXID_LOGIN_NO_DEVICE_FALLBACK=1` to opt out of the auto-fallback (you'll get the old "hang on browser callback" behavior instead). For non-interactive CI use, generate an API key with `nyxid api-key create` and authenticate via the `nyxid_ag_…` token — `nyxid login` itself short-circuits with an api-key hint when it detects `CI` / `GITHUB_ACTIONS` / `BUILDKITE` / `CIRCLECI` / `JENKINS_URL` / `GITLAB_CI`.

### Non-interactive credentials

Authenticated commands resolve credentials in this order: `--access-token`, the variable named by `--access-token-env` (default `NYXID_ACCESS_TOKEN`), `NYXID_API_KEY` as a fallback alias for the default selector, then the stored profile session. If both default variables are set, `NYXID_ACCESS_TOKEN` takes precedence. A custom `--access-token-env <VAR>` uses only `<VAR>` before falling back to the stored session.

Caller-selected credentials are fail-closed: if the server rejects one, the CLI exits nonzero and does not refresh or retry with the stored session identity.

## Agent Key login

Authorize a CLI profile with a restricted Agent Key instead of an account session:

```bash
nyxid login --agent-key --profile home-agent --base-url <BASE_URL>
```

The CLI prints a one-time code and the bare `/login/agent-key` verification URL. Enter the code and choose **Approve on this computer** to sign in and review the request, or **Approve from your phone** to display a QR code. Scan it with the NyxID mobile app, explicitly preview the request, and approve on your phone. The phone path creates no account login in the requesting computer's browser. A web browser ignores codes in the URL; enter the terminal's code explicitly.

Review the requesting device, profile, IP attribution, location, and time. Choose an existing eligible personal key or a key owned by an organization you administer, or create a new key. New keys default to `read proxy`, no allowed services or nodes, and a 90-day expiry; select the resources and expiry you intend to grant. The final confirmation shows effective permissions, resource names, allow-all warnings, expiry, and rate limits. You can reject instead of approving.

Approval issues a new login credential bound to the selected key. Selecting an existing key does not change its secret, scopes, or other consumers. The CLI receives the credential once, directly from NyxID. It stores it in the profile's `token` file with mode `0600`, plus an `auth_kind` marker, safe identity metadata in `agent_key.json`, and the backend URL. Stale account access tokens, refresh tokens, and user IDs are removed. No account session is stored, and unsupported backends fail without falling back to account login.

`nyxid whoami` and `nyxid status` identify **Authentication: Agent Key**. The key's live scopes, service and node restrictions, bindings, rate limits, and expiry remain authoritative. Credentials cannot outlive a key's expiry. Rejected credentials fail without refreshing or switching identities; `nyxid session refresh` exits with code 3 because Agent Key sessions do not refresh.

`nyxid logout --profile home-agent` attempts to revoke this login credential and always clears the local credential, reporting whether server revocation succeeded. In the web console, open the key's **Login credentials** section to revoke a specific CLI login. Revoking or rotating the key invalidates every credential issued under it. Revocation and expiry take effect on subsequent authenticated requests. Abandoned approvals expire after a 60-second delivery window and their credentials are revoked automatically.

`--agent-key` cannot be combined with `--device` or `--password`. It supports headless polling, including a human authorizing a waiting CI job; unattended jobs should normally use a pre-issued credential through the existing environment-variable options.

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
