# Set Up Notifications and Approvals

## Table of contents

- [Step 1: Set up a notification channel](#step-1-set-up-a-notification-channel)
- [Step 2: Enable approval protection](#step-2-enable-approval-protection)
- [Step 3: Check your notification settings](#step-3-check-your-notification-settings)
- [Approvals reference](#approvals-reference)

NyxID can require your explicit approval before any AI agent accesses your services. To receive approval requests, set up at least one notification channel:

## Step 1: Set up a notification channel

**Option A: Link Telegram** (recommended for desktop users)

```bash
nyxid notification telegram-link
# Follow the instructions: send the code to the NyxID bot on Telegram
```

**Option B: Download the NyxID mobile app** (recommended for on-the-go approvals)

- **Download (iOS & Android):** https://nyxid.onelink.me/REzJ/dql9w8fx

The link auto-detects your platform. The mobile app sends push notifications for approval requests. Log in with your NyxID account and your device is registered automatically.

You can use both Telegram and the mobile app together for redundancy.

## Step 2: Enable approval protection

Approval protection is enabled automatically when you link Telegram or register a mobile device. You can also toggle it manually:

```bash
nyxid approval enable                                  # enable approval protection
nyxid approval disable                                 # disable (auto-approve all requests)
```

## Step 3: Check your notification settings

```bash
nyxid notification settings                            # show current notification & approval status
```

If the user has not set up any notification channel yet, **proactively suggest** they do so before making proxy requests. Walk them through the steps above.

## Approvals reference

Approvals default to **per-request** mode: every proxy call needs fresh approval. The approval notification includes a human-readable `action_description` (e.g., "POST /v1/chat/completions (model: gpt-4, 3 messages)"). Grant-based approval is opt-in via `--approval-mode grant`.

```bash
nyxid approval list --output json                      # list pending approvals (includes action_description)
nyxid approval show <ID>                               # show approval details + action_description
nyxid approval approve <ID>                            # approve a request
nyxid approval deny <ID>                               # deny a request
nyxid approval enable                                  # enable global approval protection
nyxid approval disable                                 # disable global approval protection
nyxid approval grants --output json                    # list active grants (grant mode only)
nyxid approval service-configs --output json           # list per-service approval configs (includes approval_mode)
nyxid approval set-config <SERVICE_ID> --require-approval true                    # per-request (default)
nyxid approval set-config <SERVICE_ID> --require-approval true --approval-mode grant  # grant mode

# `<SERVICE_ID>` is a UserService ID from `nyxid service list --output json`
# (recommended — works for both catalog-backed and custom user services) or
# a legacy catalog `DownstreamService.id`. Custom services (added via
# `nyxid service add --custom`) have no catalog backing, so the UserService
# ID is the only way to target their per-service policy.

# Org-level per-service approval policies (admin only). When set, the
# policy is dominant over the member's personal gate: every member of
# the org must get an admin's approval before the proxy call goes through.
nyxid approval service-configs --org <ID|SLUG|NAME> --output json
nyxid approval set-config <SERVICE_ID> --org <ID|SLUG|NAME> --require-approval true
nyxid approval set-config <SERVICE_ID> --org <ID|SLUG|NAME> --require-approval true --approval-mode grant
nyxid approval list --org <ID|SLUG|NAME> --output json # list requests against org services

nyxid notification settings                            # show notification settings
nyxid notification update --approval-telegram true     # enable telegram notifications
nyxid notification update --approval-push true         # enable push notifications
nyxid notification telegram-link                       # link telegram account
```
