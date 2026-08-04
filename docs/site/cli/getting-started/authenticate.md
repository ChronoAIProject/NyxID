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
