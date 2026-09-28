import { serviceViewSchema } from "../src/schemas/service-view";
import { randomBytes } from "node:crypto";
import type { IncomingMessage, ServerResponse } from "node:http";
import type { Plugin } from "vite";

const READ_PATHS = new Set([
  "/api/v1/users/me",
  "/api/v1/keys",
  "/api/v1/service-insights",
  "/api/v1/api-keys",
  "/api/v1/user-services",
  "/api/v1/service-pools",
  "/api/v1/catalog",
  "/api/v1/orgs",
  "/api/v1/nodes",
  "/api/v1/runtime-config",
  "/api/v1/public/config",
  "/api/v1/providers/codex-connection",
  "/api/v1/keys/history/archived",
  "/api/v1/options/service-history-action",
]);

const UUID_SEGMENT = "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}";
const DETAIL_PATH = new RegExp(`^/api/v1/(?:keys|nodes)/${UUID_SEGMENT}$`);
const AGENT_METADATA_PATH = new RegExp(`^/api/v1/api-keys/${UUID_SEGMENT}(?:/(?:bindings|usage))?$`);
const HISTORY_PATH = new RegExp(`^/api/v1/keys/${UUID_SEGMENT}/history$`);

function isMetadataPath(path: string) {
  return READ_PATHS.has(path) || AGENT_METADATA_PATH.test(path) || DETAIL_PATH.test(path) || HISTORY_PATH.test(path) || /^\/api\/v1\/catalog\/[a-z0-9][a-z0-9-]*$/.test(path);
}

function cookie(req: IncomingMessage, name: string): string | undefined {
  return req.headers.cookie?.split(";").map((part) => part.trim())
    .find((part) => part.startsWith(`${name}=`))?.slice(name.length + 1);
}

function redirect(res: ServerResponse, location: string) {
  res.writeHead(302, { Location: location, "Cache-Control": "no-store", "Referrer-Policy": "no-referrer" });
  res.end();
}

function json(res: ServerResponse, status: number, message: string) {
  res.writeHead(status, { "Content-Type": "application/json", "Cache-Control": "no-store" });
  res.end(JSON.stringify({ message, error_code: status }));
}

/** Opt-in, loopback-only access to production metadata for the local proposal. */
export function routingPreview(backendUrl: string, frontendUrl: string): Plugin {
  const states = new Map<string, number>();
  const sessions = new Map<string, { token: string; expires: number }>();
  const sessionCookie = "nyxid_routing_preview";
  const stateCookie = "nyxid_routing_preview_state";

  return {
    name: "nyxid-local-routing-preview",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        const address = server.httpServer?.address();
        if (!address || typeof address === "string") return next();
        const origin = `http://127.0.0.1:${address.port}`;
        if (req.headers.host !== `127.0.0.1:${address.port}`) {
          return json(res, 403, "Open this preview on 127.0.0.1.");
        }
        if (req.headers.origin && req.headers.origin !== origin) {
          return json(res, 403, "Cross-origin requests are not allowed.");
        }
        const url = new URL(req.url ?? "/", origin);
        const now = Date.now();
        for (const [id, expires] of states) if (expires < now) states.delete(id);
        for (const [id, session] of sessions) if (session.expires < now) sessions.delete(id);

        if (url.pathname === "/__routing-preview/login" || url.pathname === "/login") {
          if (req.method !== "GET") return json(res, 405, "Use GET to sign in.");
          const state = randomBytes(32).toString("hex");
          states.set(state, now + 600_000);
          res.setHeader("Set-Cookie", `${stateCookie}=${state}; HttpOnly; SameSite=Lax; Path=/; Max-Age=600`);
          const login = new URL("/cli-auth", frontendUrl);
          login.searchParams.set("port", String(address.port));
          login.searchParams.set("state", state);
          login.searchParams.set("client_ua", "NyxID local routing preview (metadata only)");
          return redirect(res, login.toString());
        }

        if (url.pathname === "/callback") {
          const state = url.searchParams.get("state");
          const token = url.searchParams.get("access_token");
          if (req.method !== "GET" || !state || !states.has(state) || cookie(req, stateCookie) !== state
            || !token || token.length > 16_384 || !/^[A-Za-z0-9_.-]+$/.test(token)) {
            return json(res, 400, "Login could not be verified. Open /__routing-preview/login to try again.");
          }
          states.delete(state);
          const id = randomBytes(32).toString("hex");
          // Only the short-lived access token is retained, in memory. Refresh
          // tokens from the existing CLI callback are deliberately discarded.
          sessions.set(id, { token, expires: now + 900_000 });
          res.setHeader("Set-Cookie", [
            `${sessionCookie}=${id}; HttpOnly; SameSite=Strict; Path=/; Max-Age=900`,
            `${stateCookie}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0`,
          ]);
          return redirect(res, "/keys?view=routing");
        }

        if (url.pathname === "/api/v1/auth/logout" && req.method === "POST") {
          sessions.delete(cookie(req, sessionCookie) ?? "");
          res.setHeader("Set-Cookie", `${sessionCookie}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0`);
          return json(res, 200, "Local preview signed out.");
        }

        if (!/^\/(api|oauth|mcp)(\/|$)/.test(url.pathname) && !url.pathname.startsWith("/.well-known")) return next();
        const savingView = req.method === "PUT" && url.pathname === "/api/v1/users/me/preferences/services" && !url.search;
        if (!savingView && (req.method !== "GET" || !isMetadataPath(url.pathname))) {
          return json(res, 403, "This preview permits metadata reads and saving your service view. Service changes and execution are disabled.");
        }
        if (savingView && req.headers.origin !== origin) return json(res, 403, "Save preferences from this preview only.");
        const session = sessions.get(cookie(req, sessionCookie) ?? "");
        const isPublic = url.pathname === "/api/v1/public/config" || url.pathname === "/api/v1/runtime-config";
        if (!session && !isPublic) return json(res, 401, "Sign in to view your production connections.");
        let preferenceBody: string | undefined;
        if (savingView) {
          try {
            const chunks: Buffer[] = [];
            let size = 0;
            for await (const chunk of req) {
              const buffer = Buffer.from(chunk);
              size += buffer.length;
              if (size > 131072) return json(res, 413, "Preferences are too large.");
              chunks.push(buffer);
            }
            preferenceBody = JSON.stringify(serviceViewSchema.parse(JSON.parse(Buffer.concat(chunks).toString("utf8"))));
          } catch {
            return json(res, 400, "Invalid service view preferences.");
          }
        }
        try {
          const response = await fetch(new URL(url.pathname + url.search, backendUrl), {
            method: savingView ? "PUT" : "GET",
            headers: { ...(session ? { Authorization: `Bearer ${session.token}` } : {}), ...(savingView ? { "Content-Type": "application/json" } : {}) },
            body: preferenceBody,
            redirect: "error",
            signal: AbortSignal.timeout(20_000),
          });
          const body = await response.text();
          res.writeHead(response.status, { "Content-Type": "application/json", "Cache-Control": "no-store" });
          res.end(body);
        } catch {
          json(res, 502, savingView ? "Your default view could not be saved. Try again." : "Production data could not be loaded. Try refreshing.");
        }
      });
    },
  };
}
