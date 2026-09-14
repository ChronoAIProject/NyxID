# Durable Operation Grants

Durable operation grants authorize unattended scheduled writes without changing
the interactive `ApprovalGrant` contract. They follow least privilege and
complete mediation: authority is bound to one scheduled API key, one
`UserService`, one active published endpoint contract, bounded request values,
a finite lifetime, and finite usage quotas. NyxID revalidates all of those
properties at the proxy terminal on every invocation.

## Preview and provision

Create a JSON scope-plan request with exact `selected_service_ids`, a finite
`key_expires_at`, and one or more `selected_operations`. Each operation needs an
explicit `valid_from` so preview and confirmation are deterministic. Phase 1
accepts only endpoints explicitly classified as write with method `POST`,
`PUT`, or `PATCH`. Every path variable and required parameter must have an
`exact` or finite `one_of` constraint. JSON bodies must constrain the complete
body with the empty JSON Pointer or constrain every leaf; additional fields are
not supported.

```bash
nyxid api-key durable-plan --file durable-plan.json --output json
```

Review the returned endpoint IDs, methods, normalized paths, contract digests,
constraints, expiry, quotas, replay policies, exact service/node grants, and
owner. To provision, create a key request containing the unchanged
`selected_operations`, the returned `allowed_service_ids` and
`allowed_node_ids`, `allow_all_services: false`, `allow_all_nodes: false`, the
same `expires_at`, and the returned `normalized_grant_digest` as
`scope_plan_digest`.

```bash
nyxid api-key durable-create --file durable-create.json --yes --output json
```

NyxID recomputes the plan before mutation. Authorization, route, endpoint, or
contract drift returns a stale-plan conflict. Provisioning creates the
`scheduled_invocation` key behind a write-denied activation fence, stores all
grants, and only then enables scheduled writes. The raw key is returned once;
grant receipts contain no credential.

## Invoke

Use the scheduled key with its exact `UserService` route and provide a stable,
caller-generated operation ID. Reusing `(grant_id, operation_id)` never starts
a second downstream request.

```bash
NYXID_API_KEY='nyxid_ag_...' nyxid proxy request SERVICE_SLUG /bounded/path \
  --via-service USER_SERVICE_ID \
  --method POST \
  --data '{"exact":"authorized value"}' \
  -H 'Content-Type: application/json' \
  -H 'X-NyxID-Durable-Grant-Id: GRANT_ID' \
  -H 'X-NyxID-Operation-Id: SCHEDULE_RUN_AND_CALL_SITE_ID'
```

NyxID strips both authorization headers before forwarding. A caller-supplied
`Idempotency-Key` is also stripped. When the selected replay policy is
`downstream_idempotency_key` and the current endpoint metadata explicitly
supports it, NyxID forwards `Idempotency-Key` with the operation ID as its
value.

This is an at-most-once dispatch contract, not a universal exactly-once claim.
After a possible dispatch, transport failure is recorded as
`durable_operation_outcome_uncertain` and NyxID does not fail over to another
node. Do not retry a non-replayable write with a new operation ID. Reusing the
same ID returns the stored uncertain classification rather than dispatching.

## Path compatibility and rollout

The Google editor release changes durable path matching for **every service**,
including non-Google catalog services and operator-published endpoints without
a `proxy_operation_policy`. Ordinary REST and MCP calls use the new
canonicalization only when that service has a policy; durable grant validation
always uses it. Only the seven Google product services receive seeded policies,
but an administrator can configure a policy on any service through the services
API. This release therefore has effects outside Google Workspace.

Durable grants can now describe a custom-method template such as
`/items/{item_id}:publish`. Previously, grant planning rejected a variable next
to a literal suffix; it now recognizes the declared verb and binds only the
resource ID. The same improvement makes the shipped `llm-google-ai`
`generate_content`, `count_tokens`, and `embed_content` operations eligible for
path matching, subject to the other grant requirements.

For both existing and new grants, the forwarding path is decoded exactly once.
An ordinary parameter without an explicit path grammar rejects colons,
whitespace (including encoded spaces), and literal percent signs. For example,
`urn:example:item`, `item value`, and `100%` now fail with
`DurableGrantMismatch` (9009), even when an `exact` or `one_of` grant constraint
explicitly permits that string. The previous resolver accepted these values.
Encoding them does not bypass the check; double encoding also fails. Rejection
happens before an execution reservation, quota consumption, or provider effect.
Sheets A1 ranges have a deliberate parameter-level grammar for range colons and
quoted spaces; this does not relax other parameters. Literal `%` remains
unsupported there too.

Discord Bot's `add_reaction` publishes
`x-nyxid-path-constraint: discord_emoji` on its `emoji_name` path parameter.
The backend accepts a single Unicode emoji sequence from the Unicode 17 emoji
registry (including modifiers, flags, and joined sequences), or a custom emoji
`name:id` with `[A-Za-z0-9_]{2,32}` for the name and a positive unsigned 64-bit
snowflake written as 1–20 ASCII digits. Colons in custom emoji are encoded as
parameter data. Other parameters receive no colon permission. Multiple colons,
empty/invalid names, nonnumeric or overflowing IDs, whitespace, and literal
percent signs remain rejected. The number-sign keycap `#️⃣` has a **pre-existing**
transport limitation: `proxy_service::contains_raw_path_breaker` rejects `#`,
and `contains_percent_encoded_path_breaker` rejects `%23`. Neither guard changes
in this release. The asterisk keycap `*️⃣` works when its path value is encoded
once (`%2A%EF%B8%8F%E2%83%A3`), including with an old durable grant. This encoded
form also worked at baseline `28fd2c44`; raw `*` remains rejected by the durable
service's `normalize_path`. The two keycaps therefore have different existing
transport/normalization restrictions.

Unicode validation uses the `emojis` crate's curated sequence registry. Checking
code-point ranges or grapheme structure would admit invalid modifier/flag/joiner
combinations; a small inline list would exclude valid existing reactions. The
manifest permits `0.9.x`, and `Cargo.lock` pins `0.9.0` with its checksum. This
`no_std` crate ships generated tables and lookup code, with no build script or
build-time data download. Its licence is `(MIT OR Apache-2.0) AND Unicode-3.0`.
It adds `phf` and `phf_shared` for lookup and reuses the existing `siphasher`;
all three new packages require Rust 1.66, below the workspace's Rust 1.93 minimum.

Startup sync adds the annotation to the existing endpoint row. Durable grants
issued before that annotation continue to validate against their original
contract digest: for this exact PUT reaction template, validation also compares
the current endpoint with only the new emoji annotation removed. All remaining
contract fields and the current emoji grammar are enforced. New grants bind the
annotated contract. Existing grant and endpoint identities are retained, and no
grant rows are rewritten to refresh their digests.

Older replicas lack that legacy digest comparison, so after annotation sync
they can reject pre-annotation Discord grants with contract drift. Route these
scheduled invocations to updated replicas. Do not rerun an older replica's
startup sync: its old overlay removes the annotation, which prevents updated
replicas from validating newly issued annotated grants until the current overlay
is restored. Complete the backend upgrade before enabling schedules against the
updated catalog.

The shipped catalog was reviewed across all 31 source overlays (244 operations,
including 41 parameterized POST/PUT/PATCH operations marked as writes). The
composed Workspace spec and slug aliases reuse those operations:

| Shipped operation | Compatibility impact |
| --- | --- |
| Discord Bot `add_reaction`, `PUT /channels/{channel_id}/messages/{message_id}/reactions/{emoji_name}/@me` | Custom `name:id` emoji and supported Unicode emoji encoded once continue to work with existing and new durable grants through the explicit emoji parameter grammar. Literal MCP arguments are encoded by the tool builder; supplying a pre-encoded value creates a second encoding layer and is rejected. |
| GitHub `get_file_contents`, `GET /repos/{owner}/{repo}/contents/{path}` | Filenames can contain spaces, `%`, or `:`. This operation is read-only and cannot receive a durable grant under the current POST/PUT/PATCH write-only contract. Its ordinary proxy behavior is unchanged unless an operator configures a policy. |
| OpenAI and Mistral `models_get`, `GET /models/{model}` or `/models/{model_id}` | Fine-tuned model IDs can contain colons. These are also read-only operations and currently ineligible for durable grants. A model ID in a request body is unaffected by path validation. |
| Other pre-existing parameterized writes | The published arguments use provider IDs, repository/account names, or numeric identifiers; no further documented colon/space/percent-bearing value was identified. Most overlay schemas specify only `type: string`, so they do not prove that every provider-returned value is safe. Operator-customized endpoints and existing grant values need their own review. |

After the parameter-specific Discord fix, no shipping catalog operation is known
to regress from these path-validation changes.

The [Discord overlay](../backend/specs/catalog/discord-bot.openapi.json) explicitly
documents the custom-emoji `name:id` format. Fine-tuned model ID formats are
illustrated in the providers' [OpenAI fine-tuning guide](https://platform.openai.com/docs/guides/supervised-fine-tuning)
and [Mistral documentation](https://docs.mistral.ai/llms-full.txt).

Before upgrading a scheduler, review its durable grant path constraints and
pause or revise schedules that use unsupported values. Reauthorizing the same
unsupported value will not make it executable. Punctuation permissions belong
to the explicit Sheets range and Discord emoji parameter grammars; arbitrary IDs
remain constrained. Existing grant, endpoint, key, and service identities are
preserved; do not rewrite stored rows to bypass validation. During a mixed-version rollout,
route scheduled invocations to updated replicas for consistent enforcement;
older replicas can still accept values the updated resolver rejects.

## Manage and renew

```bash
nyxid api-key durable-grants KEY_ID --include-revoked --output json
nyxid api-key durable-revoke KEY_ID GRANT_ID --yes --output json
nyxid api-key durable-reauthorize KEY_ID --file reauthorize.json --yes --output json
```

Revocation, expiry, total quota, window quota, endpoint deactivation, contract
drift, wrong key/grant identity, and constraint mismatch fail before downstream
dispatch. Reauthorization requires a freshly previewed v2 digest, inserts the
replacement grants only after revoking prior active grants. Only personal owners
and organization admins may list or mutate grant receipts.

Stable durable error codes are `9008` through `9016`: missing, mismatch,
expired, revoked, contract drift, quota exhausted, duplicate operation,
conflicting operation reuse, and outcome uncertain, respectively.
