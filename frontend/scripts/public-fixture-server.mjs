// CI-only proxy boundary. It never signs in or sends real credentials.
import http from "node:http";
http
  .createServer((req, res) => {
    res.setHeader("Content-Type", "application/json");
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
