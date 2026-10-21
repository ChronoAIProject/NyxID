// CI-only proxy boundary. It never signs in or sends real credentials.
import http from "node:http";
http
  .createServer((req, res) => {
    res.setHeader("Content-Type", "application/json");
    if (
      req.url === "/api/v1/users/me" &&
      req.headers.cookie?.includes("fixture-auth=1")
    ) {
      res.end(
        JSON.stringify({
          id: "ci-public-user",
          email: "ci@example.invalid",
          display_name: "CI Fixture",
          is_admin: false,
          is_operator: false,
          role: "user",
          created_at: "2026-01-01T00:00:00Z",
          mfa_enabled: false,
        }),
      );
      return;
    }
    if (req.url === "/api/v1/users/me") {
      res.writeHead(401);
      res.end('{"error":"unauthorized"}');
      return;
    }
    if (req.url === "/api/v1/public/config") {
      // Real auth pages consume this typed contract; no live providers or
      // telemetry are enabled by this isolated invitation-wiring fixture.
      res.end(JSON.stringify({
        mcp_url: "http://127.0.0.1:4188/mcp",
        node_ws_url: "ws://127.0.0.1:4188/api/v1/nodes/ws",
        version: "ci-fixture",
        social_providers: [],
        invite_code_required: true,
        email_auth_enabled: true,
      }));
      return;
    }
    res.end(JSON.stringify({ fixture: true, url: req.url }));
  })
  .listen(3001, "0.0.0.0");
