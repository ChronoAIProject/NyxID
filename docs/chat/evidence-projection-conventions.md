# Evidence-Projection Conventions (Wave 2+)

One page, binding for every assistant-action postcondition surface. Derived
from the shipped Wave-1 projections (`KeyAuthorizationEvidenceResponse` in
`handlers/keys.rs`, `ApiKeyAuthorizationEvidenceResponse` in
`handlers/api_keys.rs`) and the #1464 finding they encode. The audit in
`docs/chat/assistant-waves-plan.md` §6 lists, per resource family, which
detail-response fields make the full response unusable as evidence.

## Why projections exist

The postcondition reader runs a recursive secret-shape scan over the *whole*
document it receives (`Bearer\s+\S+`, `nyxid_`-prefixed values). Detail
responses carry user-controlled free text (labels, descriptions, header
values, WS auth templates) that legitimately matches the scan, so a
legitimately configured resource would become permanently unverifiable.
Evidence is therefore served from a dedicated projection carrying exactly the
properties the reader consumes.

## Rules

1. **No user-controlled free text.** No labels, names, descriptions, header
   values, templates, error messages, URLs a user typed, or anything else a
   user or upstream service authored. IDs (UUIDs), enums, booleans, integers,
   and RFC 3339 timestamps only. (Exception on record: `ApiKey.name` is the
   irreducible remainder for agent keys — do not add more exceptions.)
2. **No `skip_serializing_if` on consumed fields.** The reader distinguishes
   explicit `null` from an absent property. A property is either always
   serialized, or governed by an explicit trio/absence rule the reader
   documents (see rule 3). Never make an existing consumed field
   conditionally absent after the fact.
3. **Lineage-trio rule.** Mutation lineage (`rotation_predecessor_id`,
   `state_version`, `updated_at`) is emitted as a trio or not at all:
   all-absent means "no lineage evidence"; present with `state_version <= 0`
   is a malformed document the reader rejects. Pre-lineage rows omit the trio
   rather than serialize zero.
4. **Absence evidence is a 404.** For delete-shaped verbs the postcondition
   read is the projection route returning 404 (body-free) — never a list
   read filtered client-side, and never a soft-deleted row with free text in
   the body.
5. **Same ACL as the detail route, strictly fewer properties.** A projection
   is mounted as `GET <detail>/authorization` (or family equivalent), uses
   the identical ownership/ACL resolution as its detail sibling, and is
   admitted to delegated `account:read` readers automatically (the delegated
   filter is deny-based, `mw/auth.rs:delegated_read_denied_path`). Only
   secret-delivering GETs need a deny entry — a conforming projection never
   is one.
6. **Additive changes only, within the verified reader constraints.** The
   upstream reader (`NyxIdApiAccessContracts.cs`, aevatar
   `feature/integrate`) parses evidence by name-addressed `JsonElement`
   reads — unknown properties are not rejected structurally — but
   `RejectSecretBearingRead` visits every added property: the name,
   normalized to ASCII-alphanumeric lowercase, must not be one of `apikey
   fullkey keyhash credential(s) accesstoken refreshtoken authorization
   cookie(s) secret(s) clientsecret password token passphrase usercode
   devicecode rawbody ...`, and no string value may match
   `Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,}`. UUIDs, RFC 3339
   timestamps, and integers all pass. Never rename, retype, or remove a
   served property.
7. **Served enum values are a closed contract.** The reader's enum parsers
   throw on unknown values (`ParseCredentialStatus`: `_ => throw`). Adding
   a new value to an enum-typed served field (e.g. `status`) is a breaking,
   coordinated change — not additive. New states need a new field or an
   upstream parser change first.
8. **Derive from the detail response type**, not from the model, so the two
   representations cannot drift (`from_key_response` /
   `from_api_key_response` pattern).
9. **Test the negative.** Every projection lands with a test asserting the
   serialized document contains no free-text carrier of its family (the §6
   audit names them) and, where lineage applies, the trio rule for both
   pre-lineage and current rows.

## Receipts (effect side of the same discipline)

Effect handlers use `services/assistant_action_receipts.rs`:
reserve-then-commit, exact-retry replay from the receipt, conflict on
identity reuse with different content, resource identity reserved before the
effect. One-time material (a full key, a raw token) is returned only by the
non-replayed response that committed it, and never stored on the receipt.
