# Reusable options API

`GET /api/v1/options/{option_set}` supplies suggestions for editable fields. The first registered set is `service-scope`; unknown sets return 404. Suggestions are optional prefill data, not an exhaustive vocabulary or a new authorization mechanism. Service-account scope create/update, token subset validation, and runtime permission checks retain their existing behavior.

## Service-account context

```http
GET /api/v1/options/service-scope?principal_type=service_account&owner_id=<owner-uuid>&limit=50
Authorization: Bearer <management-token>
```

This is a management read. Service-account credentials are rejected. Human credentials and eligible delegated reads use the existing management authentication rules, including exact `account:read` for delegated reads.

The new endpoint requires a global admin creating under their own personal owner ID, or an actual admin of the requested organization. For editing, include `service_account_id=<uuid>`: the existing account must match the owner, and the caller must be a global admin or an admin of that owning organization. This endpoint's owner validation does not change existing management routes or grant cross-owner listing.

`principal_type=service_account` is required. Owner/account IDs must be UUIDs. Optional `search` is at most 200 bytes and matches labels, values, and descriptions case-insensitively. `limit` is 1–100 (default 50), and `offset` is 0–10005 (default 0). Unknown query fields are rejected.

## Sources and response

The resolver merges two sources:

- Code-defined suggestions for existing checks: `proxy`, `llm:proxy`, `roles`, `catalog:skills:read`, and `catalog:skills:write`. The catalog skill suggestions are available before any account has used them; using them still requires a platform-admin-issued curation grant for exact catalog services.
- Scope tokens already configured on service accounts belonging to the authorized effective owner, including disabled accounts. Ownership follows `owner_user_id`, falling back to `created_by` for older records. Values are deduplicated and sorted; known definitions retain their descriptive labels. Other values use `source: "configured_scope"` and are explicitly described as previously configured custom values.

The `proxy:*` alias and `groups` can appear when configured, with descriptions of their existing behavior. UserService IDs, provider OAuth menus, API-key scope vocabularies, and operation catalogs are not sources for this menu. No exact-service permission is generated.

```json
{
  "option_set": "service-scope",
  "principal_type": "service_account",
  "owner_id": "<owner-uuid>",
  "service_account_id": null,
  "items": [{
    "value": "custom:read",
    "label": "custom:read",
    "description": "Custom scope previously configured for this owner. Its effect depends on the service handling it.",
    "group": "Previously configured scopes",
    "source": "configured_scope",
    "owner_id": "<owner-uuid>",
    "resource_id": null,
    "disabled": false,
    "disabled_reason": null
  }],
  "selected_items": [],
  "total": 1,
  "next_offset": null,
  "version": "service-account-suggestions-v2:<content-hash>",
  "freshness": {
    "definitions_version": "service-account-suggestions-v2",
    "resources": "live",
    "evaluated_at": "2026-09-17T00:00:00+00:00",
    "max_age_seconds": 0
  }
}
```

`value` is the submitted token. `selected_items` includes the edited account's current values independently of search and pagination. Custom and previously configured values remain editable; they are not disabled or limited to removal. Creating a new custom value is permitted even when it has never appeared in this endpoint. See [Service Accounts](SERVICE_ACCOUNTS.md) for existing scope semantics.

`total` counts matches. Follow `next_offset` until null. Compare `version` across pages and restart at offset zero on a change. The version hashes the full authorized suggestion content, selected values, and owner/edit context; it is independent of search and page size.

## Freshness and bounds

Every request evaluates authorization and reads current local data. Responses use `Cache-Control: private, no-store`; `evaluated_at` is the actual evaluation time. Builtin definitions are deployment-versioned constants. There is no imported metadata or artificial refresh timestamp, and no daily global job is required. A future imported source should report its actual import time/version and refresh at least daily while local ACLs remain live.

Configured-value reads project only `allowed_scopes`, with bounded cursor batches. The source accepts at most 1,000 account records, 1 MiB of scope-string content, and 10,000 distinct configured tokens per owner. A bound exceeded returns an explicit error instead of a partial menu. These are suggestion-source limits only: they do not limit stored scopes, token issuance, or custom input. Search/pagination operate on this bounded set.

`useOptions(optionSet, context)` keys the cache by signed-in identity, owner/edit context, option set, and search. Cached data is immediately stale, with revalidation on mount, focus, successful local mutations, and context changes, plus a daily polling ceiling. The hook restarts pagination on version drift. Owner changes reset cached picker labels and suggestions; existing custom selections stay visible and editable.

## Frontend and extension

`AsyncOptionSelect` is a tag multiselect with wrapping, fully visible raw-value pills and one editable combobox. Each pill has separate Edit and Remove controls, following the audit-filter interaction pattern. Click or keyboard-activate Edit to replace that value in place. Enter finishes the edit; Escape restores the original values and order. Replacements are deduplicated. Selected values are omitted from suggestions, and a prefix disappears when no unselected complete descendant remains.

While open, the dropdown automatically fetches remaining pages sequentially, with 100 items per request and a 102-page safety bound; there is no manual load-more step. Debounced remote search and page-version/context checks remain active. A transient failure on a later page keeps previously loaded choices and shows Retry. Initial/refetch failures, authorization/context failures, and invalid response data clear the menu; custom input remains available. Pending/empty/error states and keyboard/listbox interaction remain accessible.

Its optional `delimiter` enables prefix navigation; service-account scopes pass `":"`. Each displayed branch must be a prefix of an actual returned full value. For configured examples `reports:read`, `reports:finance:read`, and `reports:finance:export`, choosing `reports:` exposes its real next branches/leaves, and choosing `reports:finance:` exposes those two complete leaves. Segments from unrelated values are never combined. Remote search reloads each prefix and automatically follows its remaining pages. Branch navigation is temporary UI state, not a submitted scope or a claim of permission.

Optional `allowCustom` keeps the same input editable for complete arbitrary scopes and whitespace-separated paste. Arrow keys explicitly activate suggestions; Enter selects an active suggestion, or finishes the exact typed value when none is active. IME Enter does not finish an edit. Typed custom text updates the form value immediately, like a normal text input, while the draft stays in place until finished. Blur only closes the dropdown, so wrapping pills cannot move a Save button during its click. Automatic prefix navigation never stages an intermediate scope. Escape while adding closes suggestions and retains the typed draft; Escape while editing a pill cancels that edit. A subsequent Escape follows the containing dialog behavior. Custom input works during loading, empty results, and errors. Service-account forms enable both options and retain the space-separated string submission format through `useAppForm`.

The shared CLI create panel uses the same picker in hosted pairing and standalone mode. Standalone identity lookup uses `/users/me` through the existing authenticated proxy so suggestion caching uses the real identity. Initial identity loading preserves the active draft and keyboard focus; switching an established account or owner resets the picker context. Identity/suggestion failures have retry controls and do not prevent custom-only creation. The local proxy allows only the required exact GET routes `/users/me` and `/options/service-scope`. Rebuild the embedded wizard with `npm run build:wizard` after source changes; the Rust `wizard_bundle_freshness` test verifies its manifest/hash.

To add a set, register an explicit variant and resolver in `services/options_service.rs`, define its source service and authorized context, register dedicated OpenAPI responses, and extend the frontend context/schema types. Keep suggestions separate from domain validation; a complete domain may opt out of custom input, while an incomplete source should leave it enabled.

## Local mock preview

From `frontend/`, run `node scripts/preview-service-scopes.mjs`, then open `http://127.0.0.1:53226/scope-preview`. The development-only page uses the actual shared picker with mocked HTTP responses for available, empty, and unavailable suggestions and a local save simulation. It does not add a production route or modify real service accounts.
