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
      res.end("{}");
      return;
    }
    res.end(JSON.stringify({ fixture: true, url: req.url }));
  })
  .listen(3001, "0.0.0.0");
