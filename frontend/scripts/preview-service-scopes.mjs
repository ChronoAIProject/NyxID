import { createServer } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
let scenario = "available";
const definitions = [
  ["proxy", "All services", "Proxy access to services available to this account, including the LLM gateway."],
  ["llm:proxy", "LLM gateway", "Use the LLM gateway with this account's configured providers."],
  ["roles", "Role claims", "Include assigned roles and permissions in userinfo."],
  ["catalog:skills:read", "Read catalog skills", "Read recommendations and history for services permitted by a curation grant."],
  ["catalog:skills:write", "Manage catalog skills", "Manage recommendations for services permitted by a curation grant."],
];
const configured = [
  "reports:read", "reports:export", "reports:finance:read", "reports:finance:export",
  "pipeline:run", "deploy:staging", "deploy:production", "metrics:read", "metrics:write",
];

function reply(res, status, value) {
  res.writeHead(status, { "Content-Type": "application/json", "Cache-Control": "no-store" });
  res.end(JSON.stringify(value));
}

const server = await createServer({
  configFile: false,
  root,
  plugins: [react(), tailwindcss(), {
    name: "local-scope-preview",
    configureServer(vite) {
      vite.middlewares.use(async (req, res, next) => {
        const url = new URL(req.url, "http://localhost");
        if (url.pathname === "/scope-preview" || url.pathname === "/") {
          const html = await vite.transformIndexHtml(url.pathname, '<!doctype html><html lang="en"><head><meta charset="UTF-8"/><meta name="viewport" content="width=device-width,initial-scale=1"/><title>Service account scopes · Local preview</title></head><body><div id="root"></div><script type="module" src="/src/dev/scope-preview.tsx"></script></body></html>');
          res.writeHead(200, { "Content-Type": "text/html", "Cache-Control": "no-store" });
          res.end(html);
          return;
        }
        if (url.pathname === "/__scope_preview__/scenario" && req.method === "POST") {
          const nextScenario = url.searchParams.get("value");
          if (!["available", "empty", "unavailable"].includes(nextScenario)) {
            reply(res, 400, { message: "Unknown preview scenario" });
            return;
          }
          scenario = nextScenario;
          reply(res, 200, { scenario });
          return;
        }
        if (url.pathname === "/api/v1/options/service-scope" && req.method === "GET") {
          if (scenario === "unavailable") {
            reply(res, 503, { error: "preview_unavailable", error_code: 1000, message: "Suggestions are temporarily unavailable." });
            return;
          }
          const owner = url.searchParams.get("owner_id");
          const base = { owner_id: null, resource_id: null, disabled: false, disabled_reason: null };
          const choices = scenario === "empty" ? [] : [
            ...definitions.map(([value, label, description]) => ({ ...base, value, label, description, group: "Known permissions", source: "backend_definition" })),
            ...configured.map(value => ({ ...base, value, label: value, description: "Previously configured for this owner. Its effect depends on the service handling it.", group: "Previously configured", source: "configured_scope", owner_id: owner })),
          ];
          const search = (url.searchParams.get("search") ?? "").toLowerCase();
          const filtered = choices.filter(item => `${item.value} ${item.label} ${item.description}`.toLowerCase().includes(search));
          const offset = Math.max(0, Number(url.searchParams.get("offset")) || 0);
          const limit = Math.min(4, Math.max(1, Number(url.searchParams.get("limit")) || 4));
          reply(res, 200, {
            option_set: "service-scope", principal_type: "service_account", owner_id: owner,
            service_account_id: url.searchParams.get("service_account_id"),
            items: filtered.slice(offset, offset + limit), selected_items: [], total: filtered.length,
            next_offset: offset + limit < filtered.length ? offset + limit : null,
            version: `preview-${scenario}-${owner}`,
            freshness: { definitions_version: "preview-v1", resources: "live", evaluated_at: new Date().toISOString(), max_age_seconds: 0 },
          });
          return;
        }
        if (url.pathname.startsWith("/api/") || url.pathname.startsWith("/__scope_preview__/")) {
          reply(res, 404, { message: "This local preview only serves mocked scope suggestions." });
          return;
        }
        next();
      });
    },
  }],
  resolve: { alias: { "@": path.join(root, "src") } },
  define: { __BUILD_ID__: JSON.stringify("scope-preview") },
  server: { host: "127.0.0.1", port: Number(process.env.SCOPE_PREVIEW_PORT ?? 53226), strictPort: true },
});

await server.listen();
server.printUrls();
console.info("Open /scope-preview. All suggestions and saves are local mock data.");
