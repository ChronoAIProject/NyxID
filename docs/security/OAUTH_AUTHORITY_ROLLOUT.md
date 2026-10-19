# OAuth authority cutover

This runbook describes the authority changes under local review. Runtime
acceptance is recorded in the [W1–W4 tracker](../SECURITY_REMEDIATION_W1_W4.md).
No deployment, production revocation or client reauthorization has been
performed by this work.

## Compatibility boundary

The new online gate requires the original protected consent revision for
ordinary OAuth access and refresh tokens. OAuth delegation additionally requires
a signed OAuth origin and its matching durable JTI authority record. Derived
proxy and Aevatar callback tokens retain that source authority. Genuine
first-party service callbacks use an explicit signed Service origin.

Legacy tokens without these claims or records fail closed on new readers.
An unknown or absent origin is never interpreted as Service. Regranting consent
does not revive a token bound to an earlier revision. Clients holding legacy
grants must reauthorize; merely refreshing a legacy token is not a migration.

Old replicas neither emit the new authority nor enforce its revocation. A mixed
cluster therefore cannot promise immediate revocation. Old-to-new service
callbacks may also fail, even while their JWT lifetime remains valid. JWT key
overlap and independent HMAC preparation do not bridge this authority boundary.
This implementation has no staged legacy-token acceptance mode.

## Coordinated, drained deployment

1. Inventory every writer and verifier: OAuth authorization/code/token/refresh
   endpoints, REST and MCP admission, introspection/userinfo, proxy injection,
   callback receivers, background callback producers and independently deployed
   services that validate NyxID tokens. Record configured token/callback TTLs,
   in-flight operation bounds and offline-verifier limitations. Prepare client
   reauthorization and user-facing interruption notices before cutover.
2. Complete the applicable [HMAC and JWT preparation](KEY_LIFECYCLE.md), back up
   the matching database/configuration/key material, and verify required MongoDB
   indexes and majority/primary availability. These prerequisites preserve key
   compatibility; they do not authorize legacy OAuth tokens.
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
5. Require affected clients to complete fresh authorization and obtain newly
   bound tokens. Test revoke followed by a request on another replica, and
   revoke/regrant with an old token. Treat resulting legacy-token failures as
   expected reauthorization requirements, without copying a later consent
   revision onto an existing token.

Successful revocation applies to subsequent NyxID online admission after its
majority-acknowledged write. Already-admitted provider effects need their own
lifecycle contract. Relying parties that validate JWTs offline cannot observe
the database fence; use online introspection/admission where immediate
revocation is required, or explicitly retain their bounded expiry limitation.

## Rollback

Returning traffic to old readers restores their weaker revocation behavior and
can reject or mishandle callbacks created during the new authority epoch.
Close admission and drain new work before any coordinated rollback; retain
database evidence and the matching HMAC/JWT configuration. Do not delete JTI or
consent-revision records, relabel unknown-origin tokens, or route failed requests
to old readers as a compatibility fallback. Reauthorization and deliberate token
invalidation may be required again. Record the reduced guarantee explicitly
before reopening an older release.
