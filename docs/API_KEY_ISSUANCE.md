# API key issuance and lifetime

New ordinary API keys use empty service/node lists with both allow-all flags false.
An empty restricted list grants no resources. Callers can explicitly select broad
grants, subject to the existing owner ACL. A nonempty resource list cannot be
combined with its allow-all flag. Existing persisted model defaults and authority
are unchanged; this policy applies at creation.

## Parent and child expiry

For ordinary creation, omitted, null or empty `expires_at` means creation time plus
90 days. A future RFC3339 value or date-only value selects an explicit expiry; a
date alone means 23:59:59 UTC. `never_expires: true` explicitly selects a
nonexpiring parent and conflicts with a nonempty date. All stored expiry values
and create/rotate responses use BSON millisecond precision; a value that rounds
to an already expired instant is rejected. Use the returned `expires_at`, rather
than a client clock calculation, as the authoritative result.

Agent Key login and the restricted auth-device flow issue separate child
credentials. An omitted child expiry means the earlier of creation time plus
90 days and the parent's expiry. An explicit child expiry must be future and
within the parent's lifetime. A nonexpiring parent still receives a finite new
child by default. Selecting an existing parent does not edit its lifetime.
New-parent and child intent is resolved before retryable approval transactions;
one-time login codes persist the resolved intent when minted. Existing stored
child null expiry remains null during delivery and authentication.

Rotation retains existing expiry, resource grants and authority; it does not
restart the 90-day window or reinterpret a legacy null. Assistant action creation
uses the finite ordinary default; rotation and metadata readback preserve policy.
Managed NyxBot, conversation and internal shared transport keys deliberately
retain their own nonexpiring constructors. Public platform keys retain their
explicit finite policy. Permission-bound and scheduled invocation keys retain
exact finite expiries and the existing scope-plan digest contract.

## Client behavior

The dashboard, Agent Key login form, CLI wizard and ordinary mobile creation form
start restricted and finite. Opening/resetting a form or changing owner restores
those defaults. Explicit broad grants and nonexpiry require a fresh choice for
that owner. CLI `--expires-in-days 0` means explicit nonexpiry; omission means
90 days. Clients omit `never_expires` when false. Creation success views show the
returned expiry; an absent field from an older server is shown as unavailable.
Rotation preserves the stored lifetime; the shared wizard and terminal rotation
summaries currently omit it. Read the rotated key's details to review its expiry.

Device-code provisioning (`/devices/code/approve` and approved polling) and QR
onboard redemption create finite keys and expose their actual stored lifetime
as `api_key_expires_at`. Existing `expires_in` fields still describe the delivery
or bootstrap envelope timeout, never the API-key lifetime. Device approval shows
the default before approval and the stored value afterward.

## Deployment and compatibility

Upgrade all serving backend replicas and issuance clients, including embedded CLI
wizards, before enabling or promising this contract. During a rolling upgrade,
drain or pause the API-key creation, Agent Key/auth-device approval and one-time
login-code mint/redeem routes; resume them only after every serving replica runs
the new contract. Continue normal authentication of stored keys. Do not send
these issuance requests to mixed backend versions: old replicas retain broad
creation defaults and may interpret null expiry as nonexpiring. An action SHA or
client-side default does not make an old backend enforce the new policy.

Ordinary selections omit false `never_expires`, preserving the prior JSON shape;
old-shaped pending selections remain readable. Explicit true is a new intent in
persisted `LoginCode.selection`: old readers use `deny_unknown_fields` and reject
it. Do not mint that intent until all readers are upgraded, and do not restart
old replicas against pending selections containing it. This is an all-replica
cutover requirement, not a claim of mixed-version support or a runtime rollout
flag. Expire or redeem such pending codes under the new version before any
rollback. Existing key records require no lifetime rewrite.
