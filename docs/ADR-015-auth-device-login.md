# ADR-015: Auth-device code compatibility and approval scope

**Status:** Accepted

**Date:** 2026-09-14

Related: [issue #1535](https://github.com/ChronoAIProject/NyxID/issues/1535).
The implementation and source-fixture tests do not establish physical-device
acceptance or close the original mobile validation requirement.

## Decision

Eight-character public device codes, displayed as `XXXX-XXXX`, are the
compatibility target for installed mobile clients. Legacy account requests
continue to issue that format. Grant-capable v2 requests issue the prior
`2-XXXX-XXXX` format by default. Set `AUTH_DEVICE_EIGHT_CHAR_CODES=true` only
after the staged rollout below to issue eight-character v2 codes. Readers
accept both existing formats; the public code never authorizes a grant.

The server stores grant-capable requests in `auth_device_codes_v2` and legacy
requests in `auth_device_codes`. A shared
`auth_device_code_reservations` collection reserves the HMAC of every new
public code before its request row is inserted. For upgraded writers this makes uniqueness global
across both collections and retains the reservation through terminal-row
retention. Per-collection unique indexes remain as a second defence.

The preview response advertises `supports_grant_choice`. Clients use this
capability field rather than inferring protocol support from code length or
prefix. A legacy mobile build can therefore preview and approve an
eight-character grant-capable request as a full account session; it cannot
select or receive a restricted Agent Key because the restricted approval route
is separately authenticated and the capability is absent from legacy rows.

`/login/device` is the approval surface for a person approving another
computer, CLI, agent, or device. After identity verification it offers full
account access, an existing eligible Agent Key, or a new Agent Key with an
explicit scope. The selected result is bound to the one pending request and
is the only result the requester can poll. QR and deep links identify that
request and never carry credentials or scope decisions.

`/login` remains the ordinary full-account browser login. Its NyxID-app
shortcut intentionally uses the legacy request and browser-poll routes so
current iOS builds continue to work and a browser cannot receive a restricted
Agent Key by choosing an incidental path. A separately designed scoped
browser-session contract is required before adding a limited-session toggle;
an Agent Key secret must not be silently substituted for a browser session.

## Consequences

- After the issuance gate is enabled, existing iOS parsers can read the eight-character QR/manual codes. Marker requests still require a compatible reader.
- CLI and `/login/device` clients can negotiate restricted approval through an
  explicit capability, with no dependence on a human-code encoding.
- Rolling deployments retain protocol isolation: old replicas cannot consume
  v2 poll secrets or interpret restricted approval data as a full grant.
- A reservation can temporarily consume a code if a process crashes between
  reservation and row insertion. Its TTL is bounded by the normal ten-minute
  request lifetime plus one-day terminal retention, after which it is safe to
  reuse.

## Rollout and rollback

1. Deploy the new readers, reservation writers, lookup indexes and cleanup with
   `AUTH_DEVICE_EIGHT_CHAR_CODES=false` everywhere. V2 still uses its distinct
   nine-character marker namespace, so old eight-character legacy writers cannot
   collide with it. Public lookup checks both collections and rejects ambiguity.
2. Drain all readers that only search the legacy collection and all writers that
   do not reserve public codes. This is an operator-controlled fleet prerequisite,
   not something the reservation collection can enforce against an old binary.
3. Enable `AUTH_DEVICE_EIGHT_CHAR_CODES=true` consistently across the upgraded
   fleet. Old iOS code-only approval can then approve a v2 request as a full
   account session; the requester receives it through v2 private polling.
4. Retain old marker decoding, both request collections, reservations and indexes
   while requests and retained terminal rows drain. Failed or uncertain request
   insertion keeps its reservation until TTL; do not release after a timeout.
5. To disable new issuance, turn the gate off while keeping the compatibility
   readers. Arbitrary rollback to legacy-only readers/writers is unsafe while
   eight-character v2 rows remain live. A binary rollback requires a drained
   exchange population and a separately checked storage/index migration.

The React query and consent contract is described in
[DEVICE_LOGIN_PROTOCOL.md](DEVICE_LOGIN_PROTOCOL.md). No deployment follows from
this ADR or from the local compatibility tests.
