import { expect } from "@playwright/test";
import { test } from "../e2e/production-policy-fixture";
import fs from "node:fs";
import path from "node:path";
import { hasOnlyConfiguredEgress, observeTelemetryEgress } from "../e2e/telemetry-request";

const dist = path.resolve("dist");
const manifest = JSON.parse(fs.readFileSync(path.join(dist, "release-integrity/releases.json"), "utf8")) as {
  git_commit: string; artifacts: { role: string; path: string }[];
};

test("production response/header matrix and backend .js routing", async ({ request }) => {
  const asset = fs.readdirSync(path.join(dist, "assets")).find((name) => name.endsWith(".js"))!;
  for (const [url, status] of [["/", 200], ["/login", 200], ["/docs", 200], ["/privacy", 200], ["/terms", 200],
    ["/build-version.json", 200], [`/assets/${asset}`, 200], ["/assets/missing-policy-fixture.js", 404],
    ["/api/v1/policy-probe.js", 200], ["/oauth/policy-probe.js", 200],
    ["/api/v1/policy-probe.js?status=500", 500], ["/credential-accept/assets/missing.js", 404]] as const) {
    const response = await request.get(url, { headers: { "x-forwarded-proto": "https", host: "attacker.example" } });
    expect(response.status(), url).toBe(status);
    const headers = response.headers();
    expect(headers["x-frame-options"], url).toBe("DENY");
    expect(headers["x-content-type-options"], url).toBe("nosniff");
    expect(headers["content-security-policy"], url).toContain("frame-ancestors 'none'");
    expect(response.headersArray().filter((header) => header.name.toLowerCase() === "content-security-policy"), url).toHaveLength(1);
    expect(headers["strict-transport-security"], url).toBeUndefined();
    if (url.includes("policy-probe.js")) {
      expect((await response.json()).transport).toBe("backend");
      expect(headers["content-security-policy"]).toContain("script-src 'sha256-dGVzdA=='");
      expect(headers["content-security-policy"]).not.toContain("connect.facebook.net");
      expect(headers["referrer-policy"]).toBe("no-referrer");
    }
  }
  for (const url of ["/privacy", "/terms"]) {
    const initial = await request.get(url);
    const etag = initial.headers().etag;
    expect(etag).toBeDefined();
    const cached = await request.get(url, { headers: { "if-none-match": etag! } });
    expect(cached.status()).toBe(304);
    expect(cached.headers()["content-security-policy"]).toContain("script-src 'self'");
    const denied = await request.post(url);
    expect(denied.status()).toBe(405);
    expect(denied.headers()["content-security-policy"]).toContain("script-src 'self'");
  }
  expect((await request.get("/login?claim=fixture-bearer")).headers()["referrer-policy"]).toBe("no-referrer");
  expect((await request.get("/build-version.json")).headers()["cache-control"]).toBe("no-store");
});

test("built login submits and completes MFA under enforcing CSP", async ({ page }) => {
  let authenticated = false;
  await page.route("**/api/v1/users/me", (route) => authenticated ? route.fulfill({ json: {
    id: "11111111-1111-4111-8111-111111111111", email: "policy@example.test", display_name: "Policy fixture",
    is_admin: false, is_active: true, email_verified: true, created_at: "2026-10-19T00:00:00Z",
  } }) : route.fulfill({ status: 401, json: { error: "unauthorized", message: "Fixture session required", error_code: 1001 } }));
  let submitted = false;
  await page.route("**/api/v1/auth/login", async (route) => {
    expect(route.request().postDataJSON()).toEqual({ email: "policy@example.test", password: "fixture-only-password", client: "web" });
    submitted = true;
    await route.fulfill({ status: 401, json: { error: "mfa_required", error_code: 2002, message: "MFA required", session_token: "fixture-mfa" } });
  });
  await page.route("**/api/v1/auth/mfa/verify", async (route) => {
    expect(route.request().postDataJSON()).toEqual({ code: "123456", mfa_token: "fixture-mfa", client: "web" });
    authenticated = true;
    await route.fulfill({ status: 204 });
  });
  await page.goto("/login");
  await page.getByLabel("Email", { exact: true }).fill("policy@example.test");
  await page.getByLabel("Password", { exact: true }).fill("fixture-only-password");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("heading", { name: "MFA verification" })).toBeVisible();
  expect(submitted).toBe(true);
  await page.getByRole("textbox", { name: "Enter 6-digit verification code" }).fill("123456");
  await page.getByRole("button", { name: "Verify", exact: true }).click();
  await expect(page).toHaveURL(/\/dashboard$/);
  expect(authenticated).toBe(true);
});

test("real microphone, shipped AudioWorklet, fonts and allowed websocket", async ({ page }) => {
  await page.goto("/login");
  await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
  const worklet = fs.readdirSync(path.join(dist, "assets")).find((name) => name.startsWith("grok-audio-worklet-") && name.endsWith(".js"))!;
  const result = await page.evaluate(async (modulePath) => {
    await document.fonts.load('400 14px "Mona Sans"');
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    const audio = new AudioContext();
    await audio.audioWorklet.addModule(modulePath);
    const node = new AudioWorkletNode(audio, "nyx-grok-audio");
    const socket = await new Promise<string>((resolve, reject) => {
      const ws = new WebSocket(`${location.origin.replace("http", "ws")}/api/v1/policy-socket`);
      const timer = setTimeout(() => { ws.close(); reject(new Error("WebSocket deadline")); }, 5000);
      ws.onmessage = (event) => { clearTimeout(timer); resolve(String(event.data)); ws.close(); };
      ws.onerror = () => { clearTimeout(timer); reject(new Error("WebSocket failed")); };
    });
    const track = stream.getAudioTracks()[0]!;
    const live = track.readyState;
    stream.getTracks().forEach((item) => item.stop());
    node.disconnect();
    await audio.close();
    return { font: document.fonts.check('400 14px "Mona Sans"'), live, socket };
  }, `/assets/${worklet}`);
  expect(result).toEqual({ font: true, live: "live", socket: "connected" });
});

test("rejects injected inline scripts, event handlers and unlisted connections", async ({ page }, testInfo) => {
  for (const [directive, blocked] of [["script-src-elem", "inline"], ["script-src-attr", "inline"],
    ["connect-src", "https://unlisted.invalid/should-never-arrive"]]) {
    testInfo.annotations.push({ type: "expected-csp", description: JSON.stringify({ directive, blocked }) });
  }
  await page.goto("/login");
  const blocked = await page.evaluate(async () => {
    const events: string[] = [];
    document.addEventListener("securitypolicyviolation", (event) => events.push(event.effectiveDirective));
    const script = document.createElement("script");
    script.textContent = "window.__policyAttack = true";
    document.body.append(script);
    const button = document.createElement("button");
    button.setAttribute("onclick", "window.__policyAttack = true");
    document.body.append(button);
    button.click();
    await fetch("https://unlisted.invalid/should-never-arrive").catch(() => undefined);
    await new Promise((resolve) => setTimeout(resolve, 150));
    return { events, ran: (window as unknown as { __policyAttack?: boolean }).__policyAttack ?? false };
  });
  expect(blocked.ran).toBe(false);
  expect(blocked.events).toContain("script-src-elem");
  expect(blocked.events).toContain("script-src-attr");
  expect(blocked.events).toContain("connect-src");
});

for (const [enabled, host] of [[false, "https://us.i.posthog.com"], [true, "https://us.i.posthog.com"], [true, "https://analytics.example"]] as const) {
  test(`production PostHog ${enabled ? "enabled" : "disabled"} at ${host} has bounded egress and no lazy scripts`, async ({ page, baseURL }) => {
    await page.addInitScript((consent) => {
      // The vendor intentionally suppresses WebDriver traffic. Model the
      // supported human-browser path across both UA surfaces without changing
      // production filtering. Chromium's UA Client Hints otherwise still
      // advertise HeadlessChrome even with the configured normal Chrome UA.
      Object.defineProperty(navigator, "webdriver", { get: () => false });
      Object.defineProperty(navigator, "userAgentData", { get: () => ({
        brands: [{ brand: "Chromium", version: "151" }, { brand: "Google Chrome", version: "151" }],
        mobile: false, platform: "macOS",
      }) });
      localStorage.setItem("nyxid.telemetry_consent", JSON.stringify({ state: { asked: true, enabled: consent }, version: 1 }));
    }, enabled);
    await page.route("**/api/v1/public/config", (route) => route.fulfill({ json: {
      email_auth_enabled: true, social_providers: [], invite_code_required: false,
      telemetry_dsn: "phc_policy_fixture", telemetry_host: host, telemetry_share_analytics: false,
    } }));
    const requests: string[] = [];
    const appOrigin = new URL(baseURL!).origin;
    page.on("request", (request) => {
      observeTelemetryEgress(requests, request.url(), request.resourceType(), appOrigin, host);
    });
    await page.route("https://*.posthog.com/**", (route) => route.fulfill({ json: { status: 1 }, headers: { "access-control-allow-origin": "*" } }));
    await page.route("https://analytics.example/**", (route) => route.fulfill({ json: { status: 1 }, headers: { "access-control-allow-origin": "*" } }));
    await page.goto("/login");
    await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
    if (enabled) await expect.poll(() => requests.length).toBeGreaterThan(0);
    await page.waitForTimeout(700);
    expect(requests.some((url) => /\.js(?:\?|$)|\/config|\/flags/.test(url))).toBe(false);
    expect(hasOnlyConfiguredEgress(requests, host), JSON.stringify(requests)).toBe(true);
    expect(await page.locator('script[src*="posthog"]').count()).toBe(0);
    if (!enabled) expect(requests).toEqual([]);
  });
}

test("standalone credential page keeps its policy and packaged SRI assets", async ({ page, request }) => {
  const url = "/api/v1/nodes/fixture/credentials/pending/fixture/accept";
  const response = await request.get(url);
  expect(response.headers()["content-security-policy"]).toContain("sha384-");
  expect(response.headers()["content-security-policy"]).not.toContain("connect.facebook.net");
  expect(response.headersArray().filter((h) => h.name.toLowerCase() === "content-security-policy")).toHaveLength(1);
  await page.goto(url);
  await expect(page.locator("#credential-accept-root")).not.toBeEmpty();
  for (const artifact of manifest.artifacts.filter((a) => a.role === "credential_accept_script")) {
    const asset = await request.get(artifact.path);
    expect(asset.status()).toBe(200);
    expect(asset.headers()["content-type"]).toContain("javascript");
  }
  expect(manifest.git_commit).not.toBe("unknown");
});

test("legal pages execute the external theme asset under the same CSP", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("nyxid.theme", JSON.stringify({ state: { mode: "light" } })));
  for (const url of ["/privacy", "/terms"]) {
    await page.goto(url);
    await expect(page.locator("html")).toHaveClass(/theme-light/);
    expect(await page.locator("script:not([src])").count()).toBe(0);
  }
});
