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
