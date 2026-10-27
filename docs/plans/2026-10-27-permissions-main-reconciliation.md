# Permissions current-main reconciliation assessment

Read-only Astra assessment, 2026-10-27. Reviewer: 01a17a9b-5082-7b83-8171-2e791e9fd01e.
Pinned Permissions input: 7185b5ef7eb56194b2e1cbf42f045536a9cb7847.
Examined main: 49f5a61214f6a29485471adea8185763e5ef424d.
Route change: d734bec28edf3409424a952caa97475c9c066c37 (#2049).
Coordinator accepted this direction, not implementation approval. No source edit, merge, reconciliation or additional backend test was performed.

## Bounded resolution

Main adds an explicit authorization path for a guest's exact owner-established model route. Retaining F-001's unconditional guest model-generation rejection after that path would break the intentionally authorized case. In the later isolated reconciliation, preserve main's early guest_inference check and guest_model::require before the id=None return; remove only the now-redundant unconditional guest rejection after those defenses are retained. Keep Permissions' bounded client_metadata classification. Metadata establishes no authority.

Preserve main's owner-only model_transport = !chat.guest && generation, raw_inference, consent/bootstrap handling and remember_owner/model_attempt. Do not copy the Permissions expression whose owner restriction was only implicit in the earlier blanket denial. Keep Admission::revalidate's guest route check before egress.

The guest path requires a live, nonstopped persisted conversation matching owner/agent/conversation/key/turn and guest status; a nonempty generation model; closed stateless request; no method override or pool; and current server input admission. It compares the configured profile, catalog service, connection, semantic operation (model/endpoint/query/headers), execution target and durable credential identity/epoch. selection_fingerprint intentionally removes the per-turn input token, while require separately validates/injects that exact current server token. remember_owner requires owner authority, input admission, an active model_attempt and a matching configured profile, and fences persisted owner-turn state transactionally.

## Existing exact regression coverage to retain

All names below are source-identified; no current-main execution is claimed.

- handlers::assistant_nyxagent::tests::guest_turn_tests::guest_1995_model_callback_through_real_turn_pipeline: actual owner/guest callback pipeline, catalog route, service approval refusal, stateful refusal and concurrency. Successful guest calls preserve route identity and create no owner model consent; refusal paths assert zero provider requests.
- handlers::assistant_nyxagent::tests::guest_turn_tests::guest_1995_owner_model_control: owner control.
- handlers::assistant_nyxagent::tests::guest_turn_tests::guest_1995_nyxbot_model_works_but_guest_tool_calls_stay_refused: bound model transport does not authorize NyxBot guest tools.
- handlers::assistant_nyxagent::tests::guest_turn_tests::guest_1995_route_binding_rejects_other_byok_models_and_unadmitted_input: unrelated catalog/BYOK connection, different or empty model, wrong endpoint, store=true, method override, missing input, new guest thread and unbound catalog path.
- handlers::assistant_nyxagent::tests::guest_turn_tests::guest_1995_never_learns_a_route_from_a_guest: repeated unbound callbacks do not establish a route, followed by actual owner establishment and successful guest reuse.
- services::assistant_service_execution::tests::guest_1995_route_binding_uses_durable_credential_identity: turn/token refresh compatibility and explicit credential-epoch replacement mismatch.
- services::assistant_service_execution::tests::permissions::codex_callback_guest_does_not_gain_owner_model_consent: original default-Use/generic-approval-disabled unbound regression; retain HTTP403 and zero provider/consent/card assertions. It must remain explicitly unbound after reconciliation.
- services::assistant_service_execution::tests::permissions::codex_metadata_does_not_admit_stateful_provider_or_unknown_effects and services::assistant_service_execution::tests::permissions::codex_callback_projected_read_path_keeps_exact_negative_approvals: closed metadata, stateful/hosted/unknown negative classification and exact permission behavior, including the seven-field replacement-key case.

## Missing interaction proof only

Extend the actual existing turn/callback harness to establish the route through the owner path and exercise valid 1121-byte metadata, alongside its existing plain callback control. Assert exactly the intended provider effect and zero new consent/cards/route change. Exercise the new metadata negative cases through that bound route so unknown/replacement metadata and stateful/provider-hosted effects cannot use it. Reuse existing route-mismatch coverage; add only missing stale-route/admission-to-egress, spoofed owner-route establishment and relevant changed-input/target/credential/semantic-header assertions where the existing exact tests do not already demonstrate them.

Preserve owner override destructive exact approval and Permissions denial/cancellation checks. Combined I-05/I-06 remain separately OPEN and must execute against the final combined identity. Actual callback/deployed-version/Lark-Web and #2007 readiness/five-operation live acceptance remain OPEN. No broad suite duplication is required just to restate unchanged coverage.

## Ownership

Root serializes integration/main imports. The existing Automation Sol remains the integration source writer until the coordinator explicitly assigns isolated reconciliation scope to the same Permissions Sol. Original Permissions source remains pinned during its narrow batch. This assessment is neither a branch edit nor a release authorization.
