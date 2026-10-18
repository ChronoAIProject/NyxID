import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer } from "node:http";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import { checkPreview, previewOptions } from "./preview-credits.mjs";

const options = previewOptions([], { BACKEND_URL: "http://127.0.0.1:3001" });
const unauthorized = { error: "unauthorized", error_code: 1001 };
const json = (body, status = 200) => new Response(JSON.stringify(body), {
  status, headers: { "content-type": "application/json" },
});

function backend(overrides = {}) {
  return async url => {
    const pathname = new URL(url).pathname;
    if (overrides[pathname]) return overrides[pathname](new URL(url));
    if (pathname === "/health") return json({ status: "ok" });
    if (pathname === "/api/v1/public/config") return json({ email_auth_enabled: true, social_providers: [] });
    return json(unauthorized, 401);
  };
}

test("matching protected APIs and email login pass configuration checks", async () => {
  assert.deepEqual((await checkPreview(options, backend())).failures, []);
});

test("missing summary routes block an otherwise healthy backend", async () => {
  const result = await checkPreview(options, backend({
    "/api/v1/admin/credits/summary": () => new Response(null, { status: 404 }),
    "/api/v1/billing/credits": () => new Response(null, { status: 404 }),
  }));
  assert.equal(result.failures.length, 2);
  assert.ok(result.failures.every(failure => failure.includes("HTTP 404")));
});

test("SPA fallbacks and anonymous success never prove protected API readiness", async () => {
  const result = await checkPreview(options, backend({
    "/api/v1/admin/credits/summary": () => new Response("<html>App</html>"),
    "/api/v1/billing/credits": () => json({ available_micros: "0" }),
  }));
  assert.equal(result.failures.length, 2);
});

test("a remote social callback and rejected localhost return URL block launch", async () => {
  const result = await checkPreview(options, backend({
    "/api/v1/public/config": () => json({ email_auth_enabled: false, social_providers: ["google"] }),
    "/api/v1/auth/social/google": () => new Response(null, { status: 302, headers: {
      location: "https://accounts.google.com/o/oauth2/v2/auth?redirect_uri=https%3A%2F%2Fapi.example.test%2Fcallback",
      "set-cookie": "nyx_social_return_to=; Max-Age=0; HttpOnly",
    } }),
  }));
  assert.equal(result.failures.length, 2);
  assert.ok(result.failures.some(failure => failure.includes("cookie host")));
  assert.ok(result.failures.some(failure => failure.includes("rejected the local return URL")));
});

test("same cookie host and an accepted exact return URL pass social configuration", async () => {
  const result = await checkPreview(options, backend({
    "/api/v1/public/config": () => json({ email_auth_enabled: false, social_providers: ["google"] }),
    "/api/v1/auth/social/google": url => new Response(null, { status: 302, headers: {
      location: "https://accounts.google.com/o/oauth2/v2/auth?redirect_uri=http%3A%2F%2F127.0.0.1%3A3001%2Fcallback",
      "set-cookie": `nyx_social_return_to=${encodeURIComponent(url.searchParams.get("return_to"))}; Max-Age=600; HttpOnly`,
    } }),
  }));
  assert.deepEqual(result.failures, []);
});

test("unreachable backend produces failures, never a ready state", async () => {
  const result = await checkPreview(options, async () => { throw new Error("offline"); });
  assert.equal(result.checks.length, 0);
  assert.equal(result.failures.length, 4);
});

test("unknown or missing authentication configuration blocks launch", async () => {
  const result = await checkPreview(options, backend({
    "/api/v1/public/config": () => json({ social_providers: [] }),
  }));
  assert.ok(result.failures.some(failure => failure.includes("invalid sign-in configuration")));
});

test("preview options reject credentials, arbitrary options and non-loopback binds", () => {
  assert.throws(() => previewOptions([], { BACKEND_URL: "https://secret@example.test" }));
  assert.throws(() => previewOptions(["--host", "0.0.0.0"], {}));
  assert.throws(() => previewOptions(["--port", "NaN"], {}));
  assert.throws(() => previewOptions(["--open"], {}));
  assert.equal(previewOptions(["--port", "4614"], {}).localOrigin, "http://127.0.0.1:4614");
});

test("the actual launcher exits before build/server startup against an older backend", async () => {
  const server = createServer((req, res) => {
    res.setHeader("content-type", "application/json");
    if (req.url === "/health") return res.end(JSON.stringify({ status: "ok" }));
    if (req.url === "/api/v1/public/config") return res.end(JSON.stringify({ email_auth_enabled: true, social_providers: [] }));
    res.writeHead(404); res.end();
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  try {
    const address = server.address();
    await assert.rejects(promisify(execFile)(process.execPath, [fileURLToPath(new URL("./preview-credits.mjs", import.meta.url))], {
      env: { ...process.env, BACKEND_URL: `http://127.0.0.1:${address.port}` }, timeout: 15_000,
    }), error => {
      assert.equal(error.code, 1);
      assert.match(error.stderr, /Preview was not started/);
      assert.doesNotMatch(error.stdout, /vite.*building|Local:/);
      return true;
    });
  } finally {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
});
