# Device login

Use the server-returned verification URL and public code. The requester retains
its private polling secret. Scanning, previewing and signing in are not approval.
The human reviews the request, selects permitted account/Agent Key access, then
explicitly approves the actual grant. A requester requiring restricted access uses
`/auth/agent-key`; it must never silently fall back to full account authority.

The implemented browser hints are `user_code`, `login_type`, `key_source`,
`key_name`, `expiry_days`, `platform`, `permissions`, `services`, and
`service_permissions`. They are bounded editable suggestions, never authority.
No URL parameter supplies owner IDs, resource UUIDs, allow-all grants or credentials.
See the repository [device login protocol](https://github.com/ChronoAIProject/NyxID/blob/main/docs/DEVICE_LOGIN_PROTOCOL.md)
for the exact allowlist, safe identity return, consent snapshots and source pointers.

Eight-character v2 codes require the staged server issuance gate documented in
[ADR-015](https://github.com/ChronoAIProject/NyxID/blob/main/docs/ADR-015-auth-device-login.md).
Do not assume every deployed backend already issues that format or can roll back
to legacy-only readers. Normal browser login remains account-only.

Effective options include per-key credential overrides and owner-bound platform
services. The durable current/future platform grant is always extra access;
unreported provider scopes cannot be described as exact. New browser approval
submits consent snapshots for explicit and currently implied connections. Filters
never downscope provider credentials.
