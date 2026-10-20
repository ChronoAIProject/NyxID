# Tool Catalog: naming, reuse of provider patterns, CLI, and roles

Status: decision note, 2026-10-08. Companion to `TOOLS_REGISTRY_PLAN.md` and `PORT_COVERAGE.md`.

## Naming

| Vendor | User-facing term | Grouping term | Admin surface |
| --- | --- | --- | --- |
| MCP / Anthropic | tools | servers, connectors | — |
| OpenAI | tools, connectors | apps | Actions (GPTs) |
| Composio, Arcade | tools | toolkits | tool catalog |
| Zapier, Pipedream | actions | apps | integrations |
| Monid | tools | providers | catalog |

Recommendation: user entry **Tools** (items are "tools", grouped by "provider"); admin entry **Tool Catalog** with Providers / Tools / Imports tabs. Drop "Registry" from product names; NyxID code already uses "registry" for the embedded overlay registry and "catalog" for `DownstreamService` rows, so Tool Catalog names the thing it edits. Keep **AI Services** for the user's own connections.

## Existing provider patterns to reuse

| Pattern | Where it lives | How Tools use it |
| --- | --- | --- |
| Seeded catalog rows with `service_category` | `provider_service.rs` seeds | Tool rows are `internal` rows: master credential, `requires_user_credential = false` |
| Overlay spec key shared by several slugs | `catalog_spec_registry.rs` | Tool variant of an existing provider (`tools-x` beside `api-twitter`) without a second spec |
| Startup upsert of endpoints by name | `catalog_spec_sync.rs` | Importer writes through the same path; admin-added rows are never deleted |
| Spec-URL auto-discovery | admin create/update with `openapi_spec_url` | Vendors with a native spec need no overlay |
| Platform key audience and auto-provisioning | `PlatformKeyConfig`, `unified_key_service` | "Ready to use" for every user or an allowlist, no vendor connection |
| Platform-key pricing lane | `ServiceBilling.platform_key_pricing` | Per-request price, billed to the acting person; allowances for free caps |
| Operation policy and risk annotations | `proxy_operation_policy`, `x-aevatar-tool` | Publication gate; readOnly/destructive for grants and confirmations |
| Scoped keys and agent grants | `allow_auto_connected_services`, specialist grants | Which AIs may call a tool |
| Overlay drift guard | `.github/workflows/catalog-spec-drift.yml` | Add each vendor that publishes an upstream spec |
| Catalog skill editor roles | `nyxid:catalog:skills:read/write`, SA `catalog:skills:*` | Template for a service-editor role |

## CLI

Today `nyxid service add --catalog-admin` and `nyxid service update --catalog-admin` create and edit catalog rows against the admin `/services` routes: slug, name, base URL, auth method, credential from env, platform key enabled/audience/allow/deny, BYOK and platform-key prices and components, inference metadata.

```bash
nyxid service add --catalog-admin --slug tools-x --label "X public search" \
  --endpoint-url https://api.x.com/2 --auth-method bearer --credential-env X_APP_BEARER \
  --platform-key-enabled true --platform-key-audience public \
  --platform-key-metric requests --platform-key-price 0.002
```

Missing for the tool workflow, all backed by routes that already exist except the last two:

| Command | Backing route | Status |
| --- | --- | --- |
| `--openapi-spec-url`, `--service-category`, `--topics`, `--offering-kind` on add/update | `POST/PUT /services` | flags to add |
| `nyxid catalog endpoint list/add/update/disable <service>` | `/services/{id}/endpoints[/{id}]` | new subcommand |
| `nyxid catalog discover <service>` | `/services/{id}/discover-endpoints` | new subcommand |
| `nyxid catalog spec import --file overlay.json --slug ...` | new overlay store route | new |
| `nyxid catalog publish <service> [--operation name]...` | new publication route (flips active + policy) | new |

## Roles

`POST/PUT /services` currently requires the platform admin role. Mirror the catalog-skills pattern:

- Add `nyxid:catalog:services:read` and `nyxid:catalog:services:write` role scopes, assignable to humans through existing role management, and `catalog:services:read/write` token scopes for service accounts.
- Catalog editors can create and edit tool rows, endpoints, labels, overlays, and publication of priced-or-free tools that are already credentialed.
- Platform admins keep: entering or rotating master credentials, enabling a platform key or widening its audience, and authoring prices. These are secrets and money.
- The CLI needs no new auth: a human login with the role, or a service account with the token scope, both already work with `--catalog-admin`.

## Decision (2026-10-08)

Naming follows monid.ai: user entry **Tools** at `/tools`, admin entry **Tools** at `/admin/tools`; items are tools, grouped by provider and category, using Monid's category vocabulary from the audit as the starting list.

## CLI status for the four tool-authoring tasks

| Task | CLI today | Gap |
| --- | --- | --- |
| Add a new catalog service | Yes: `nyxid service add --catalog-admin` (slug, name, base URL, auth method, credential env, platform key, prices) | `--openapi-spec-url` is sent on update but not on create; no `--service-category` flag (backend accepts it) |
| Define endpoints | No. `nyxid endpoint` manages the user's own endpoints, not catalog `ServiceEndpoint` rows. Routes exist: list/create/update/delete under `/services/{id}/endpoints` and `/discover-endpoints` | New `nyxid catalog endpoint list/add/update/disable` and `nyxid catalog discover`; interim path is setting a spec URL and letting auto-discovery run |
| Set pricing | Yes: `--platform-key-metric requests --platform-key-price <credits>` or `--platform-key-free`, BYOK lane, `--platform-key-component <metric>=<price>` | None; price is per service, so one price point per catalog row |
| Grouping or tags | No. The model has no tags or topics field; nearest are `service_category`, `visibility`, `capabilities`, `recommended_skills` | Additive `topics: Vec<String>` and `offering_kind` on `DownstreamService`, accepted by create/update, exposed as `--topic` (repeatable) and `--offering-kind` |
