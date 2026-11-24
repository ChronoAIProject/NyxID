import type { Plugin } from "vite";

function isTemplateReturn(value: string | null, origin: string, cookies = "") {
  if (!value || !URL.canParse(value, origin)) return false;
  const target = new URL(value, origin);
  const setup = target.searchParams.get("skillSetup");
  const skill = target.searchParams.get("skill");
  return (
    target.origin === origin &&
    target.pathname === "/assistant" &&
    !target.hash &&
    Array.from(target.searchParams).length === 2 &&
    setup !== null &&
    /^[0-9a-f]{32}$/.test(setup) &&
    skill !== null &&
    /^[a-z0-9-]+$/.test(skill) &&
    cookies.split(/;\s*/).includes(`nyxid_skill_setup_${setup}=${skill}`)
  );
}

// Social providers redirect to the configured HTTPS backend. For this
// loopback preview, finish hosted SSO there and let the local browser
// receive its own session through NyxID's existing device-login protocol.
export function templateSignInPreview(): Plugin {
  return {
    name: "template-sign-in-preview",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        const address = server.httpServer?.address();
        if (!address || typeof address === "string") return next();
        const url = new URL(req.url ?? "/", "http://localhost");
        if (
          !/^\/api\/v1\/auth\/social\/(apple|github|google)$/.test(url.pathname)
        )
          return next();
        const allowedHosts = ["127.0.0.1", "nyxbot-preview.localhost"].map(
          (host) => `${host}:${address.port}`,
        );
        if (
          req.method !== "GET" ||
          !allowedHosts.includes(req.headers.host ?? "")
        ) {
          res.writeHead(403).end();
          return;
        }
        if (
          !isTemplateReturn(
            url.searchParams.get("return_to"),
            `http://${req.headers.host}`,
            req.headers.cookie,
          )
        ) {
          return next();
        }
        const html = await server.transformIndexHtml(
          "/__preview/template-sign-in",
          `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="referrer" content="no-referrer"><title>NyxID preview sign-in</title></head>
<body><div id="root"></div><script type="module" src="/src/dev/template-sign-in-preview.tsx"></script></body></html>`,
        );
        res.writeHead(200, {
          "Content-Type": "text/html; charset=utf-8",
          "Cache-Control": "no-store",
        });
        res.end(html);
      });
    },
  };
}
