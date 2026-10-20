// @vitest-environment node
import { createServer, type Server } from "node:http";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { Connect, ViteDevServer } from "vite";
import { routingPreview } from "../dev/routing-preview";

function listen(server: Server): Promise<string> {
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => {
    const address = server.address();
    if (address && typeof address !== "string") resolve(`http://127.0.0.1:${address.port}`);
  }));
}

function close(server: Server): Promise<void> {
  return new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
}

describe("local preview metadata gateway", () => {
  let local: Server;
  let upstream: Server;
  let origin: string;
  const calls: { path?: string; authorization?: string; cookie?: string; method?: string; body?: string }[] = [];

  beforeAll(async () => {
    upstream = createServer(async (req, res) => {
      const chunks: Buffer[] = [];
      for await (const chunk of req) chunks.push(Buffer.from(chunk));
      calls.push({ path: req.url, authorization: req.headers.authorization, cookie: req.headers.cookie, method: req.method, body: Buffer.concat(chunks).toString("utf8") });
      res.setHeader("Content-Type", "application/json");
      res.end(JSON.stringify({ keys: [] }));
    });
    const backend = await listen(upstream);
    let middleware: Connect.NextHandleFunction;
    local = createServer((req, res) => middleware(req, res, () => { res.writeHead(404); res.end(); }));
    const plugin = routingPreview(backend, "https://nyx.example");
    const configure = plugin.configureServer as (server: ViteDevServer) => void;
    configure({ httpServer: local, middlewares: { use: (handler: Connect.NextHandleFunction) => { middleware = handler; } } } as unknown as ViteDevServer);
    origin = await listen(local);
  });

  beforeEach(() => calls.splice(0));
  afterAll(async () => { await Promise.all([close(local), close(upstream)]); });

  async function startLogin() {
    const response = await fetch(`${origin}/__routing-preview/login`, { redirect: "manual" });
    const target = new URL(response.headers.get("location")!);
    expect(target.origin + target.pathname).toBe("https://nyx.example/cli-auth");
    expect(target.searchParams.get("port")).toBe(new URL(origin).port);
    return { state: target.searchParams.get("state")!, cookie: response.headers.get("set-cookie")!.split(";")[0]! };
  }

  async function signIn() {
    const login = await startLogin();
    const params = new URLSearchParams({ state: login.state, access_token: "test.access.token", refresh_token: "ignored.refresh.token" });
    const response = await fetch(`${origin}/callback?${params}`, { headers: { Cookie: login.cookie }, redirect: "manual" });
    expect(response.status).toBe(302);
    expect(response.headers.get("location")).toBe("/keys?view=routing");
    const sessionCookie = response.headers.getSetCookie()[0]!;
    expect(sessionCookie).toContain("HttpOnly");
    expect(sessionCookie).not.toContain("test.access.token");
    return sessionCookie.split(";")[0]!;
  }

  it("requires a session for account metadata", async () => {
    expect((await fetch(`${origin}/api/v1/keys`)).status).toBe(401);
    expect(calls).toHaveLength(0);
  });

  it("accepts a bound login callback and forwards the access token only to allowed metadata reads", async () => {
    const cookie = await signIn();
    const response = await fetch(`${origin}/api/v1/keys`, { headers: { Cookie: cookie } });
    expect(response.status).toBe(200);
    expect(calls).toEqual([{ path: "/api/v1/keys", authorization: "Bearer test.access.token", cookie: undefined, method: "GET", body: "" }]);
  });

  it.each([
    "/api/v1/keys/13ae3c40-5ec0-4eee-9e20-25c60209dd12",
    "/api/v1/catalog/llm-openai",
    "/api/v1/nodes/13ae3c40-5ec0-4eee-9e20-25c60209dd12",
    "/api/v1/api-keys",
    "/api/v1/service-insights?ids=13ae3c40-5ec0-4eee-9e20-25c60209dd12",
    "/api/v1/api-keys/13ae3c40-5ec0-4eee-9e20-25c60209dd12",
    "/api/v1/api-keys/13ae3c40-5ec0-4eee-9e20-25c60209dd12/bindings",
    "/api/v1/service-pools",
    "/api/v1/providers/codex-connection",
    "/api/v1/keys/history/archived",
    "/api/v1/options/service-history-action",
    "/api/v1/keys/13ae3c40-5ec0-4eee-9e20-25c60209dd12/history",
  ])("preserves metadata access for the original detail pages: %s", async (path) => {
    const cookie = await signIn();
    expect((await fetch(`${origin}${path}`, { headers: { Cookie: cookie } })).status).toBe(200);
    expect(calls[0]?.path).toBe(path);
  });

  it("rejects an unbound callback even with a valid state", async () => {
    const login = await startLogin();
    expect((await fetch(`${origin}/callback?state=${login.state}&access_token=test.token`, { redirect: "manual" })).status).toBe(400);
  });

  it("rejects replay of a consumed callback", async () => {
    const login = await startLogin();
    const url = `${origin}/callback?state=${login.state}&access_token=test.token`;
    const options = { headers: { Cookie: login.cookie }, redirect: "manual" as const };
    expect((await fetch(url, options)).status).toBe(302);
    expect((await fetch(url, options)).status).toBe(400);
  });

  it.each([
    ["POST", "/api/v1/keys"],
    ["DELETE", "/api/v1/keys/id"],
    ["POST", "/api/v1/service-pools"],
    ["POST", "/api/v1/providers/codex-connection/verify"],
    ["PUT", "/api/v1/service-pools/id/members"],
    ["DELETE", "/api/v1/service-pools/id"],
    ["GET", "/api/v1/proxy/s/openai/models"],
    ["GET", "/api/v1/keys/id/reveal"],
    ["GET", "/api/v1/keys/13ae3c40-5ec0-4eee-9e20-25c60209dd12/reveal"],
    ["GET", "/api/v1/nodes/13ae3c40-5ec0-4eee-9e20-25c60209dd12/pending-credentials"],
    ["GET", "/api/v1/catalog/llm-openai/endpoints"],
    ["GET", "/oauth/authorize"],
    ["GET", "/mcp"],
  ])("blocks %s %s before it reaches production", async (method, path) => {
    const cookie = await signIn();
    expect((await fetch(`${origin}${path}`, { method, headers: { Cookie: cookie } })).status).toBe(403);
    expect(calls).toHaveLength(0);
  });

  it("rejects cross-origin reads even with a local session", async () => {
    const cookie = await signIn();
    const response = await fetch(`${origin}/api/v1/keys`, { headers: { Cookie: cookie, Origin: "https://other.example" } });
    expect(response.status).toBe(403);
    expect(calls).toHaveLength(0);
  });

  const preferences = { search: "team", organization_ids: ["org-1", "org-2"], service_group_ids: ["catalog:openai", "catalog:codex"], source: "org", state: "enabled", service_type: "http", show_auto_connected: false };

  it("forwards only validated service preferences under the preview user's identity", async () => {
    const cookie = await signIn();
    const response = await fetch(`${origin}/api/v1/users/me/preferences/services`, {
      method: "PUT", headers: { Cookie: cookie, Origin: origin, "Content-Type": "application/json" }, body: JSON.stringify(preferences),
    });
    expect(response.status).toBe(200);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ path: "/api/v1/users/me/preferences/services", method: "PUT", authorization: "Bearer test.access.token" });
    expect(JSON.parse(calls[0]!.body)).toEqual(preferences);
    expect(calls[0]?.cookie).toBeUndefined();
  });

  it("rejects unauthenticated, cross-origin, malformed and oversized preference writes", async () => {
    const cookie = await signIn();
    for (const [headers, body, status] of [
      [{ Origin: origin }, JSON.stringify(preferences), 401],
      [{ Cookie: cookie }, JSON.stringify(preferences), 403],
      [{ Cookie: cookie, Origin: "https://other.example" }, JSON.stringify(preferences), 403],
      [{ Cookie: cookie, Origin: origin }, JSON.stringify({ ...preferences, user_id: "other" }), 400],
      [{ Cookie: cookie, Origin: origin }, JSON.stringify({ ...preferences, state: "healthy" }), 400],
      [{ Cookie: cookie, Origin: origin }, "x".repeat(131073), 413],
    ] as const) {
      const response = await fetch(`${origin}/api/v1/users/me/preferences/services`, { method: "PUT", headers, body });
      expect(response.status).toBe(status);
    }
    expect(calls).toHaveLength(0);
  });

  it("clears only the local session on logout", async () => {
    const cookie = await signIn();
    expect((await fetch(`${origin}/api/v1/auth/logout`, { method: "POST", headers: { Cookie: cookie, Origin: origin } })).status).toBe(200);
    expect((await fetch(`${origin}/api/v1/keys`, { headers: { Cookie: cookie } })).status).toBe(401);
    expect(calls).toHaveLength(0);
  });
});
