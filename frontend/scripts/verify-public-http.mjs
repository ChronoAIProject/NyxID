import fs from "node:fs/promises";
import assert from "node:assert/strict";
const base = process.env.PUBLIC_TEST_ORIGIN || "http://127.0.0.1:4188";
const manifest = JSON.parse(
  await fs.readFile("dist/public-release.json", "utf8"),
);
const report = [];
async function check(url, status, verify) {
  const response = await fetch(base + url, { redirect: "manual" });
  assert.equal(response.status, status, `${url}: ${response.status}`);
  assert.equal(response.headers.get("x-content-type-options"), "nosniff", url);
  assert.equal(response.headers.get("x-frame-options"), "DENY", url);
  assert.ok(response.headers.get("referrer-policy"), url);
  if (verify) await verify(response);
  report.push({ url, status: "pass", httpStatus: status });
  return response;
}
for (const page of manifest.pages) {
  await check(page.path, 200, async (r) => {
    assert.match(r.headers.get("content-type"), /text\/html/);
    assert.equal(r.headers.get("cache-control"), "no-cache");
    assert.ok((await r.text()).includes("<h1"));
  });
  await check(page.markdown, 200, async (r) => {
    assert.match(
      r.headers.get("content-type"),
      /text\/markdown; charset=utf-8/,
    );
    assert.equal(r.headers.get("x-robots-tag"), "noindex");
    assert.ok(
      (await r.text()).includes(
        `canonical: https://nyx.chrono-ai.fun${page.path}`,
      ),
    );
  });
}
for (const url of [
  "/not-a-route",
  "/solutions/unknown",
  "/blog/luna-bistro",
  "/blog/push-approvals-walkthrough",
  "/docs/unknown",
  "/preview/draft",
  "/zh/docs",
  "/admin/unknown/path",
  "/settings/nonexistent/path",
  "/read/en/unknown.md",
  "/index.html",
  "/app.html",
  "/public-routing.conf",
  "/__public/unknown.html",
])
  await check(url, 404);
for (const url of [
  "/assistant",
  "/keys/fixture-id",
  "/orgs/fixture-org/service-accounts/fixture-sa",
  "/admin/users/fixture-user",
  "/login?code=INV&return_to=%2Fkeys",
  "/login/device?user_code=fixture",
  "/onboarding/link?claim=fixture",
  "/start?t=help-desk",
])
  await check(url, 200, async (r) => {
    assert.equal(r.headers.get("cache-control"), "no-store");
    assert.match(r.headers.get("x-robots-tag"), /noindex/);
  });
for (const [url, target] of [
  [
    "/security?lang=zh&code=INV&return_to=%2Fkeys",
    "/zh/security?code=INV&return_to=%2Fkeys",
  ],
  [
    "/zh/security/?topic=bundle&lang=en&utm_source=test",
    "/security?topic=bundle&utm_source=test",
  ],
  ["/security?code=INV&lang=zh", "/zh/security?code=INV"],
  ["/privacy.html?x=1", "/privacy?x=1"],
  ["/docs/", "/docs"],
])
  await check(url, 301, (r) => assert.equal(r.headers.get("location"), target));
await check("/security?lang=en&lang=zh", 400);
await check("/security?lang=zh&LANG=en", 400);
await check("/security?LANG=zh", 301, (r) =>
  assert.equal(r.headers.get("location"), "/zh/security"),
);
await check("/security?lang=invalid", 200);
await check("/llms.txt", 200, async (r) => {
  assert.match(r.headers.get("content-type"), /text\/plain/);
  assert.match(await r.text(), /^# NyxID/);
});
for (const url of [
  "/api/__probe?code=fixture",
  "/api/v1/proxy/fixture/config.conf",
  "/oauth/__probe?state=fixture",
  "/.well-known/__probe",
  "/mcp/__probe",
  "/health",
])
  await check(url, 200, async (r) => assert.equal((await r.json()).url, url));
const privacyProbe = await fetch(
  base + "/security?code=nyxid-ci-log-privacy-check&return_to=%2Fkeys",
  { headers: { Referer: base + "/?code=nyxid-ci-log-privacy-check" } },
);
assert.equal(privacyProbe.status, 200);
const page = await check("/security", 200);
const etag = page.headers.get("etag");
assert.ok(etag);
assert.equal(
  (await fetch(base + "/security", { headers: { "If-None-Match": etag } }))
    .status,
  304,
);
await fs.writeFile(
  "dist/public-http-validation.json",
  JSON.stringify({ status: "pass", checks: report }, null, 2),
);
console.log(`Public HTTP gate passed: ${report.length} serial checks.`);
