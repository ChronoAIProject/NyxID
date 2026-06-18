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

If `nyxid login` can't open a browser (SSH session, container, WSL without `$DISPLAY`), it auto-falls back to the **device-code flow**: the CLI prints a one-time code + a URL, you open the URL on any signed-in browser (phone, laptop), type the code, approve, and the CLI completes. You can also force the flow explicitly:

```bash
nyxid login --device --base-url <BASE_URL>
```

Set `NYXID_LOGIN_NO_DEVICE_FALLBACK=1` to opt out of the auto-fallback (you'll get the old "hang on browser callback" behavior instead). For non-interactive CI use, generate an API key with `nyxid api-key create` and authenticate via the `nyxid_ag_…` token — `nyxid login` itself short-circuits with an api-key hint when it detects `CI` / `GITHUB_ACTIONS` / `BUILDKITE` / `CIRCLECI` / `JENKINS_URL` / `GITLAB_CI`.

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
