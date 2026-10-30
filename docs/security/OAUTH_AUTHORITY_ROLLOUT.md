# OAuth authority cutover

This runbook describes the shipped OAuth authority checks and the coordinated
deployment required to activate them across participating writers and verifiers.
See [implemented security controls](../SECURITY_REMEDIATION_W1_W4.md) for related
changes and remaining limits.

## Compatibility boundary

The new online gate requires the original protected consent revision for
ordinary OAuth access and refresh tokens. OAuth delegation additionally requires
a signed OAuth origin and its matching durable JTI authority record. Derived
proxy and Aevatar callback tokens retain that source authority. Genuine
first-party service callbacks use an explicit signed Service origin.

Ordinary browser approval is an add-only review: it unions the approved scope and
service selections with the live, unexpired consent and preserves that consent's
expiry. Equal authority is a no-op. Pure broadening keeps the existing consent
revision so other sessions remain valid; each token still carries and enforces its
own scope, service, and resource claims. Narrowing, expiry shortening, revocation,
regrant, and authority-removing Machine or binding changes rotate the revision.

Legacy access tokens without a protected consent reference fail closed on new
readers; clients must refresh them. A legacy OAuth refresh record can upgrade
without reauthorization only when all of these server-side checks pass:

- The record has no `protected_consent`, is unrevoked and is unexpired; its
  issuing OAuth client is still active.
- A live consent exists for the same user and client. The refresh record's
  stored `created_at` is **strictly later than `granted_at + 60 seconds`**.
  Equality and the entire 60-second ambiguity window require reauthorization.
- The successor binds that live consent revision and never widens the legacy
  record's authority. Its scopes are the legacy scopes (or the historical
  first-party fallback when absent) intersected with the consent's scopes,
  preserving legacy order and removing `machine`; an empty result requires
  reauthorization. Machine authority requires the Machine consent flow.
  Its service restriction intersects both allowlists: allow-all survives only
  when both allow all, one restricting side supplies its IDs, and two restricting
  sides supply their intersection. A legacy consent with `allow_all_services`
  false and a null service allowlist contributes an empty allowlist: identity
  scopes survive but no service is reachable, and any retained resource URI
  rejects that upgrade without consuming the record. Legacy resource URIs are
  otherwise retained. Every URI
  must resolve and identify a service authorized by the intersected allowlist
  (the NyxID MCP URI remains service-neutral); otherwise the upgrade fails before
  any write and leaves the legacy record unconsumed. The protected reference is
  validated before issuance and fenced again before returning authority.

The legacy record is consumed exactly once. Replaying it fails, including within
the normal bound-token rotation retry grace period. Concurrent upgrades have one
winner. Revoke deletes consent and revokes its refresh records; incremental
replacement advances `granted_at`. A record predating either new grant cannot
acquire its revision. An old replica's record created after a regrant can upgrade
only if it passes the same strict 60-second cutoff. Neither JWT `iat` nor a
client-provided timestamp substitutes for the stored creation time. First-party
refresh records (`Uuid::nil()` client) retain their existing session checks.

Legacy delegation and callback tokens without authenticated origin/authority
remain fail-closed and require the coordinated drain below. An unknown or absent
origin is never interpreted as Service. Regranting consent never revives a token
bound to an earlier revision. Refresh upgrade does not repair delegation or
callback authority.

Old replicas neither emit the new authority nor enforce its revocation. A mixed
cluster therefore cannot promise immediate revocation. Old-to-new service
callbacks may also fail, even while their JWT lifetime remains valid. JWT key
overlap and independent HMAC preparation do not bridge this authority boundary.
Access, delegation and callback verification have no legacy acceptance mode;
the eligible ordinary refresh-record upgrade above is the compatibility path.

## Coordinated, drained deployment

1. Inventory every writer and verifier: OAuth authorization/code/token/refresh
   endpoints, REST and MCP admission, introspection/userinfo, proxy injection,
   callback receivers, background callback producers and independently deployed
   services that validate NyxID tokens. Record configured token/callback TTLs,
   in-flight operation bounds and offline-verifier limitations. Prepare client
   refresh/reauthorization handling and user-facing interruption notices before
   cutover; distinguish eligible ordinary refresh records from delegation and
   callback tokens that must drain.
2. Complete the applicable [HMAC and JWT preparation](KEY_LIFECYCLE.md), back up
   the matching database/configuration/key material, and verify required MongoDB
   indexes and majority/primary availability. These prerequisites preserve key
   compatibility; they do not establish consent or callback authority.
3. Stop new OAuth issuance, refresh, exchange and execution admission on **all**
   old replicas, including internal routes and background producers. Drain
   already-admitted operations and their callbacks using the deployment's
   bounded operation deadlines. Cancel or explicitly reconcile work that cannot
   drain. Waiting only for an access-token TTL does not drain longer operations
   or refresh authority. No old writer may remain reachable behind a different
   ingress or restart from an autoscaling image.
4. While admission remains closed, replace every relevant writer/verifier with
   the new release. Verify matching configuration, JTI storage/index readiness,
   bounded primary linearizable authority reads and majority security writes.
   Exercise direct OAuth, delegation, MCP, introspection/userinfo and derived
   callbacks across independent replicas. Confirm legacy/unknown-origin tokens
   are rejected and first-party service callbacks still work. Reopen only after
   all participating replicas are ready; do not perform an ordinary mixed
   rolling rollout and advertise the new guarantee during that interval.
5. Let eligible ordinary OAuth clients refresh once to obtain bound tokens under
   the server-record cutoff above. Require fresh authorization for missing,
   expired, revoked or ambiguous legacy records. Recreate drained delegation
   flows from a currently authorized, bound source. Test a qualifying upgrade
   and its rejected replay, revoke followed by a
   request on another replica, and revoke/regrant with an old token. Never copy
   a later consent revision onto an ineligible record or an existing access token.

Successful revocation applies to subsequent NyxID online admission after its
majority-acknowledged write. Already-admitted provider effects need their own
lifecycle contract. A timeout or database failure during online authority
validation returns HTTP 503 with `Retry-After: 1` (`oauth_authority_unavailable`
for application responses, `temporarily_unavailable` at the OAuth token
endpoint). Clients retain their tokens and retry the request; an unavailable
authority store is not evidence that authorization changed. Only a definitive
authority denial requires authorization again.

Relying parties that validate JWTs offline cannot observe
the database fence. A correctly signed, unexpired token can remain accepted by
such a verifier after consent revocation or client deletion, even with the new
release and a valid key overlap. Use online introspection/admission where
immediate revocation is required. Otherwise document the configured maximum
token lifetime (including verifier clock leeway) as that relying party's
revocation delay; JWT rotation and JWKS publication do not remove this limit.

## Rollback

Returning traffic to old readers restores their weaker revocation behavior and
can reject or mishandle callbacks created during the new authority epoch.
Close admission and drain new work before any coordinated rollback; retain
database evidence and the matching HMAC/JWT configuration. Do not delete JTI or
consent-revision records, relabel unknown-origin tokens, or route failed requests
to old readers as a compatibility fallback. Reauthorization and deliberate token
invalidation may be required again. Record the reduced guarantee explicitly
before reopening an older release.

## Diagnosing early OAuth sign-outs

Use read-only queries to distinguish consent authority churn from an ordinary
session failure. The exact collection names are `refresh_tokens`,
`authorization_codes`, `audit_log`, and `sessions`.
Run these against a read-only MongoDB role and select only counts and bounded
metadata:

```javascript
// Live refresh rows whose protected revision differs from the live consent.
db.refresh_tokens.aggregate([
  {$match:{revoked:false,expires_at:{$gt:new Date()},"protected_consent.consent_revision":{$type:"string"}}},
  {$lookup:{from:"consents",let:{u:"$user_id",c:"$client_id"},pipeline:[{$match:{$expr:{$and:[{$eq:["$user_id","$$u"]},{$eq:["$client_id","$$c"]}]}}}],as:"consent"}},
  {$match:{$expr:{$ne:["$protected_consent.consent_revision",{$arrayElemAt:["$consent.revision",0]}]}}},
  {$group:{_id:"$client_id",count:{$sum:1}}}
]);

// Authorization codes issued but never exchanged, bounded to a recent window.
db.authorization_codes.aggregate([
  {$match:{used:false,created_at:{$gte:ISODate("2026-10-15")},expires_at:{$gt:new Date()}}},
  {$group:{_id:{client_id:"$client_id",hour:{$dateTrunc:{date:"$created_at",unit:"hour"}}},count:{$sum:1}}}
]);

// Existing audit records for code issuance by client and day.
db.audit_log.aggregate([
  {$match:{event_type:"oauth_code_issued",created_at:{$gte:ISODate("2026-10-15")}}},
  {$group:{_id:{client_id:"$event_data.client_id",day:{$dateTrunc:{date:"$created_at",unit:"day"}}},count:{$sum:1}}}
]);

// MFA rejection counts for correlation with deployment and MFA enablement.
db.audit_log.aggregate([
  {$match:{event_type:"mfa_unassured_session_rejected",created_at:{$gte:ISODate("2026-10-15")}}},
  {$group:{_id:{$dateTrunc:{date:"$created_at",unit:"hour"}},count:{$sum:1}}}
]);
```

The consent writer emits the low-cardinality `OAuth consent authority updated`
signal with `outcome` (`created`, `no_op`, `broadened`, `narrowed`, or
`regranted`) and a
client identifier. Protected-reference failures emit the structured server-log record `Protected
OAuth authority rejected` with a route class (`access`, `refresh`, or `mcp`) and
a reason such as `revision_mismatch`, `consent_missing`, `scope_not_granted`,
`expired`, `client_inactive`, or `legacy_unbound`; query those records in the deployment's log aggregator by client
and time bucket. Authorization-code issuance is represented by the existing
`OauthAuthorizationGranted` telemetry event and the `oauth_code_issued` audit
row. The new consent/rejection signals contain no tokens, codes, redirect URLs, or scope text.
The existing issuance audit retains its established grant metadata and adds no
parallel event or extra audit volume; each decision still emits one issuance event.
Protected rejections are structured logs rather than extra audit inserts. For
JSON server logs, this read-only query groups their attribution by client,
route, reason and hour without exporting any credential-bearing data:

```sh
jq -s '[.[] | select(.fields.message == "Protected OAuth authority rejected")]
  | group_by([.fields.client_id, .fields.route_class, .fields.reason, .timestamp[0:13]])
  | map({client_id: .[0].fields.client_id, route_class: .[0].fields.route_class,
         reason: .[0].fields.reason, hour: .[0].timestamp[0:13], count: length})' nyxid.jsonl
```

Unused-code counts describe outstanding live codes, including legitimate
in-progress sign-ins; TTL cleanup limits historical diagnosis. Sample these
counts over time alongside issuance audits. Correlate them with
refresh `invalid_grant`, retryable authority 503s, refresh-reuse warnings, and
MFA rejection audits; a reload that restores identity points to a transient
SPA 401 rather than a revoked NyxID session.

### Stable consent approvals and CAS

The ordinary screen lists this request's scopes and service selection, not a
complete editable historical grant. Approval therefore unions with the live
unexpired consent inside its transaction; only the submitted scope and service
selection (plus required resources) enter the new code. Allow-all on an older
consent does not widen a newly restricted code. Equal authority performs no
consent write or Machine invalidation. Service IDs are irrelevant to equality
when both grants allow all. Ordinary review preserves a live expiry; an expired
grant contributes no authority to a new approval.

Pure broadening keeps the authority revision and issuance fence. `granted_at`
advances for an actual broadening to preserve the legacy unbound-refresh
60-second eligibility cutoff; a true no-op preserves it. REST/userinfo, refresh,
MCP and proxy checks retain each token's original scope, service allowlist,
allow-all and resource claims and intersect them with live consent. Machine
contracts also bind their own policy, consent reference, source security epochs
and live service restrictions. A fresh Machine approval or binding replacement
retains its existing consumption/version gates. No token is rebound.

The optional, serde-defaulted `write_version` changes on consent authority
writes independently of the authority revision. CAS additionally compares old
authority fields for rolling writers that omit the additive version. Issuance
markers remain authority-neutral and excluded from review fingerprints; they
serialize with transactional revocation without invalidating other refreshes.
Old readers ignore the new field and new readers accept absent fields; no
backfill is needed. Ordinary requests bind a durable transactional revocation generation in the
signed request, checked against the same consent transaction. Revocation rotates
that generation, so even a request with a skewed clock cannot resurrect it; a
fresh request after revoke can authorize immediately, including in the same
second. Old signed requests without the additive field use a conservative
`iat <= revoked_at` compatibility fence (same-second requests restart review).
Double submit before revocation is a no-op and can issue another working callback.

### Browser delivery and native handoff

Both SPA decision forms (including their Machine selection) send the hidden
`browser_form=true` field. No mobile, CLI, or SDK code posts directly to these
decision routes: they enter the browser authorization flow instead. API examples
and test callers retain 302 + Location; explicit JSON Accept retains the prior
contract. The form DTO accepts unknown fields, so new forms remain compatible
with old replicas. Fetch Metadata and HTML Accept cover older forms served from
a cached frontend. Login redirects also use the interstitial for a browser POST
to keep one completion contract even when frontend/backend origins differ.

The shared branded success page preserves the two-second native/loopback delay,
message, close-tab guidance and visible Continue link. An immediate meta refresh
completes other consent POSTs. Chromium, Firefox and Safari treat meta refresh
as document navigation after the 200 ends the form submission; `form-action`
therefore does not govern its redirect chain. External protocol launches can
still require user activation or an OS/browser confirmation: the visible link
supplies an explicit user-initiated handoff when automatic navigation is declined.
No script is required for that fallback. nginx preserves the upstream CSP for
these HTML responses; the SPA CSP stays unchanged.

### SPA 401 route inventory

| SPA call / backend route family | Non-session 401 source and disposition |
| --- | --- |
| `/proxy/s/*` and `/proxy/{service_id}/*` (including assistant direct integrations) | Downstream credentials or upstream HTTP 401 are intentionally passed through; changing them would break proxy contracts. Existing integration transports also opt into `preserveSessionOn401`. |
| `/llm/*` | Provider HTTP 401 remains part of the OpenAI-compatible upstream response contract. |
| `/auth/login`, `/auth/register`, recovery, `/auth/refresh`, `/auth/mfa/verify`, `/auth/mfa/cancel` | The submitted login, MFA challenge or refresh credential can fail independently of the cookie. Existing exclusions remain. MFA policy/status behavior is owned separately. |
| `/providers/*` and `/keys/*` connection/device/token flows | External OAuth errors use their existing provider error contracts; no additional direct provider-to-session 401 mapping was found. Any propagated 401 now gets identity confirmation. |
| `/delegation/*`, `/mcp/config` and credential-specific execution | A credential's live authority can fail independently of the browser cookie; preserve the authentication contract and confirm browser identity. |
| `/users/me` | Authoritative identity 401 clears immediately. |

No incorrect source status was found that could be changed while preserving
these public contracts. Other API 401s trigger one debounced, single-flight
identity request. Only its 401 clears auth; 5xx/network failures retain it, and
the caller authority guard runs again after the await. Account changes also
fence the debounce result and the eventual global-state mutation.
