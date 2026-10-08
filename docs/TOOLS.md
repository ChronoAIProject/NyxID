# Tools

Tools are catalog services offered through NyxID-held credentials or without authentication. `/tools` groups them by supplier; `/admin/tools` manages providers and operation publication. AI Services (`/keys`) retains BYOK bindings of tool rows while hiding platform bindings.

Phase 1 management requires a platform admin. New operations on tool rows default to inactive drafts; only published, active operations execute through HTTP, WebSocket and MCP. Legacy endpoints default to published. Rejected tool operations return HTTP 404 / code 12600. MCP names remain `{service_slug}__{endpoint.name}`.

Tool rows must be internal and provider-less when using a master credential. Existing credential storage, platform-key ACLs and billing lanes remain authoritative. Topics come from `/api/v1/tools/topics`. No tool rows are seeded.

Phase 2 will add runtime OpenAPI overlay storage, editor roles, Imports, and catalog spec commands.
