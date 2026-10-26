import { expect } from "@playwright/test";
import { test } from "../e2e/production-policy-fixture";
import { mockDashboard, managedBootstrap } from "../e2e/managed-onboarding-fixtures";
import fs from "node:fs";
import path from "node:path";

function fixture(name: string): Buffer {
  const directory = process.env.NYXID_TEST_FIXTURE_DIR ?? path.dirname(process.env.NYXID_TEST_FONT!);
  return fs.readFileSync(path.join(directory, name));
}

test("managed WhatsApp rejects an unlisted runtime API origin", async ({ page }, testInfo) => {
  const origin = "https://unlisted-api.example";
  testInfo.annotations.push({ type: "expected-csp", description: JSON.stringify({ directive: "connect-src", blocked: `${origin}/api/v1/channel-bots/managed-onboarding/whatsapp/complete` }) });
  await mockDashboard(page);
  await page.route("**/api/v1/runtime-config", (route) => route.fulfill({ json: {
    api_base_url: origin, release_integrity: { enabled: false, manifest_url: null, verification_ttl_secs: 300 },
  } }));
  await page.route("**/channel-bots/managed-onboarding/whatsapp", (route) => route.fulfill({ json: managedBootstrap }));
  let attempts = 0;
  await page.route(`${origin}/**`, (route) => { attempts++; return route.fulfill({ status: 500 }); });
  await page.route("https://connect.facebook.net/en_US/sdk.js", (route) => route.fulfill({
    contentType: "text/javascript", body: `window.FB={init(){},login(callback){callback({authResponse:{code:"fixture-code"}});window.dispatchEvent(new MessageEvent("message",{origin:"https://www.facebook.com",data:JSON.stringify({type:"WA_EMBEDDED_SIGNUP",event:"FINISH",data:{waba_id:"444",phone_number_id:"333",business_id:"555"}})}));}};`,
  }));
  await page.goto("/channel-bots?connect=whatsapp");
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Label", { exact: true }).fill("Runtime policy fixture");
  await dialog.getByRole("button", { name: "Connect with Meta" }).click();
  await expect(dialog.getByText("Failed to fetch", { exact: true })).toBeVisible();
  expect(new URL(page.url()).pathname).toBe("/channel-bots");
  expect(attempts).toBe(0);
});

test("explicit additional WebSocket origin handshakes; unlisted origin is blocked", async ({ page }, testInfo) => {
  const origin = process.env.NYXID_TEST_EXTRA_WS_ORIGIN ?? "ws://127.0.0.1:18489";
  const denied = "ws://localhost:18489/policy-socket";
  testInfo.annotations.push({ type: "expected-csp", description: JSON.stringify({ directive: "connect-src", blocked: denied }) });
  await page.goto("/login");
  const received = await page.evaluate(async (url) => new Promise<string>((resolve, reject) => {
    const ws = new WebSocket(`${url}/policy-socket`);
    const timer = setTimeout(() => { ws.close(); reject(new Error("Additional origin handshake deadline")); }, 5000);
    ws.onmessage = ({ data }) => { clearTimeout(timer); ws.close(); resolve(String(data)); };
    ws.onerror = () => { clearTimeout(timer); reject(new Error("Additional origin handshake failed")); };
  }), origin);
  expect(received).toBe("connected");
  const rejected = await page.evaluate(async (url) => new Promise<boolean>((resolve) => {
    const ws = new WebSocket(url);
    ws.onerror = () => { ws.close(); resolve(true); };
    ws.onopen = () => { ws.close(); resolve(false); };
  }), denied);
  expect(rejected).toBe(true);
});

for (const status of [401, 503]) {
  test(`one-shot MFA refresh ${status} recovers without reusing a consumed challenge`, async ({ page }) => {
    let verified = false;
    let refreshWorks = false;
    const tokens: unknown[] = [];
    let loginCount = 0;
    await page.route("**/api/v1/users/me", (route) => refreshWorks ? route.fulfill({ json: {
      id: "11111111-1111-4111-8111-111111111111", email: "policy@example.test", display_name: "Policy fixture",
      is_admin: false, is_active: true, email_verified: true, created_at: "2026-10-19T00:00:00Z",
    } }) : route.fulfill({ status: verified ? status : 401,
      json: { error: "session_unavailable", message: "Fixture session unavailable", error_code: 1001 } }));
    await page.route("**/api/v1/auth/login", (route) => {
      loginCount++;
      return route.fulfill({ status: 401, json: {
        error: "mfa_required", error_code: 2002, message: "MFA required", session_token: `fixture-retry-mfa-${loginCount}`,
      } });
    });
    await page.route("**/api/v1/auth/mfa/verify", async (route) => {
      const body = route.request().postDataJSON();
      const consumed = tokens.some((item) => (item as { mfa_token: string }).mfa_token === body.mfa_token);
      tokens.push(body);
      if (consumed) {
        await route.fulfill({ status: 401, json: { error: "unauthorized", error_code: 1001, message: "Consumed challenge" } });
        return;
      }
      verified = true;
      await route.fulfill({ status: 204 });
    });
    await page.goto("/login");
    await page.getByLabel("Email", { exact: true }).fill("policy@example.test");
    await page.getByLabel("Password", { exact: true }).fill("fixture-only-password");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await page.getByRole("textbox", { name: "Authenticator or recovery code" }).fill("123456");
    await page.getByRole("button", { name: "Verify", exact: true }).click();
    expect(new URL(page.url()).pathname).not.toBe("/dashboard");
    if (status === 401) {
      await expect(page.getByLabel("Email", { exact: true })).toBeVisible();
      await page.getByLabel("Email", { exact: true }).fill("policy@example.test");
      await page.getByLabel("Password", { exact: true }).fill("fixture-only-password");
      await page.getByRole("button", { name: "Sign in", exact: true }).click();
      await page.getByRole("textbox", { name: "Authenticator or recovery code" }).fill("123456");
    } else {
      await expect(page.getByRole("alert")).toHaveText("Verification failed. Please try again.");
      await expect(page.getByRole("textbox", { name: "Authenticator or recovery code" })).toBeDisabled();
    }
    refreshWorks = true;
    await page.getByRole("button", { name: status === 401 ? "Verify" : "Retry session check", exact: true }).click();
    await expect(page).toHaveURL(/\/dashboard$/);
    expect(tokens).toEqual(Array.from({ length: status === 401 ? 2 : 1 }, (_, index) => ({ code: "123456", mfa_token: `fixture-retry-mfa-${index + 1}`, client: "web" })));
  });
}

for (const sessionIssued of [true, false]) {
  test(`uncertain MFA POST checks session before retry; issued=${sessionIssued}`, async ({ page }) => {
    let attempted = false;
    let posts = 0;
    await page.route("**/api/v1/users/me", (route) => attempted && sessionIssued ? route.fulfill({ json: {
      id: "11111111-1111-4111-8111-111111111111", email: "policy@example.test", display_name: "Policy fixture",
      is_admin: false, is_active: true, email_verified: true, created_at: "2026-10-19T00:00:00Z",
    } }) : route.fulfill({ status: 401, json: { error: "unauthorized", message: "No session", error_code: 1001 } }));
    await page.route("**/api/v1/auth/login", (route) => route.fulfill({ status: 401, json: {
      error: "mfa_required", error_code: 2002, message: "MFA required", session_token: "uncertain-one-shot",
    } }));
    await page.route("**/api/v1/auth/mfa/verify", async (route) => { posts++; attempted = true; await route.abort("failed"); });
    await page.goto("/login");
    await page.getByLabel("Email", { exact: true }).fill("policy@example.test");
    await page.getByLabel("Password", { exact: true }).fill("fixture-only-password");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await page.getByRole("textbox", { name: "Authenticator or recovery code" }).fill("123456");
    await page.getByRole("button", { name: "Verify", exact: true }).click();
    if (sessionIssued) await expect(page).toHaveURL(/\/dashboard$/);
    else await expect(page.getByLabel("Email", { exact: true })).toBeVisible();
    expect(posts).toBe(1);
  });
}

test("actual Telegram loader uses bound cross-origin messages without eval, spoofing or replay", async ({ page, context }) => {
  await mockDashboard(page);
  await page.route("**/api/v1/providers", (route) => route.fulfill({ json: { providers: [{
    id: "telegram-fixture", name: "Telegram fixture", slug: "telegram", description: "Policy fixture",
    provider_type: "telegram_widget", credential_mode: "admin", has_oauth_config: true,
    is_active: true, default_scopes: [], supports_pkce: false,
  }] } }));
  await page.route("**/api/v1/providers/my-tokens**", (route) => route.fulfill({ json: { tokens: [] } }));
  await page.route("**/api/v1/llm/status", (route) => route.fulfill({ json: { providers: [], gateway_url: "" } }));
  await page.route("**/api/v1/providers/telegram-fixture/connect/telegram", (route) => route.fulfill({ json: { bot_username: "nyxid_policy_fixture_bot" } }));
  let loaderRequests = 0;
  await context.route("https://telegram.org/js/telegram-widget.js?22", async (route) => {
    loaderRequests++;
    await route.fulfill({ contentType: "text/javascript", body: fixture("telegram-widget-v22.js") });
  });
  await context.route("https://oauth.telegram.org/**", (route) => route.fulfill({
    contentType: "text/html", body: "<!doctype html><title>Telegram transport boundary</title><button>Log in</button>",
  }));
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => { finish = resolve; });
  const callbacks: unknown[] = [];
  await page.route("**/api/v1/providers/telegram-fixture/connect/telegram/callback", async (route) => {
    callbacks.push(route.request().postDataJSON());
    await pending;
    await route.fulfill({ json: { message: "Connected" } });
  });
  await page.goto("/providers");
  await page.getByRole("button", { name: "Login with Telegram", exact: true }).click();
  const dialog = page.getByRole("dialog");
  const iframe = dialog.locator('iframe[src^="https://oauth.telegram.org/"]');
  await expect(iframe).toHaveCount(1);
  expect(loaderRequests).toBe(1);
  const widgetScript = dialog.locator("script");
  await expect(widgetScript).toHaveCount(1);
  await expect(widgetScript).toHaveAttribute("src", "https://telegram.org/js/telegram-widget.js?22");
  await expect(widgetScript).toHaveAttribute("data-telegram-login", "nyxid_policy_fixture_bot");
  await expect(widgetScript).toHaveAttribute("data-size", "large");
  // Bind the actual consumer attributes: the upstream auth-url/HTML callbacks,
  // post and discussion branches remain outside this login-only contract.
  expect(await widgetScript.evaluate((element) => Array.from(element.attributes)
    .map((attribute) => attribute.name).filter((name) => name.startsWith("data-")).sort()))
    .toEqual(["data-size", "data-telegram-login"]);
  const frameURL = new URL((await iframe.getAttribute("src"))!);
  expect(frameURL.protocol).toBe("https:");
  expect(frameURL.origin).toBe("https://oauth.telegram.org");
  expect(frameURL.pathname).toBe("/embed/nyxid_policy_fixture_bot");
  expect(frameURL.searchParams.get("origin")).toBe(new URL(page.url()).origin);
  expect(frameURL.searchParams.get("return_to")).toBe(page.url());
  const payload = JSON.stringify({ event: "auth_user", auth_data: {
    id: 123456, first_name: "Policy fixture", auth_date: 1792359000, hash: "a".repeat(64),
  } });
  // Real message from another frame at the allowed origin must be rejected.
  await page.evaluate(() => {
    const frame = document.createElement("iframe");
    frame.id = "wrong-telegram-frame";
    frame.src = "https://oauth.telegram.org/policy-wrong-frame";
    document.body.append(frame);
  });
  const wrongFrame = await page.locator("#wrong-telegram-frame").elementHandle();
  const wrongContext = await wrongFrame!.contentFrame();
  await wrongContext!.waitForLoadState();
  await wrongContext!.evaluate((data) => parent.postMessage(data, "*"), payload);
  // A well-framed event with the owned WindowProxy but a wrong origin must
  // also be rejected. The actual cross-origin success below uses postMessage.
  await iframe.evaluate((element, data) => window.dispatchEvent(new MessageEvent("message", {
    origin: "https://telegram.org", source: (element as HTMLIFrameElement).contentWindow, data,
  })), payload);
  await page.waitForTimeout(100);
  expect(callbacks).toEqual([]);
  const owned = await (await iframe.elementHandle())!.contentFrame();
  await owned!.evaluate((data) => {
    parent.postMessage("invalid-json", "*");
    parent.postMessage(JSON.stringify({ event: "auth_user", auth_data: {} }), "*");
    parent.postMessage(data, "*");
    parent.postMessage(data, "*");
  }, payload);
  await expect.poll(() => callbacks.length).toBe(1);
  await expect(dialog.getByText("Verifying your Telegram identity...").first()).toBeVisible();
  await page.waitForTimeout(150);
  expect(callbacks).toEqual([JSON.parse(payload).auth_data]);
  finish();
  await expect(dialog.getByText("Your Telegram account has been connected successfully.")).toBeVisible();
  await dialog.getByRole("button", { name: "Done" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await wrongContext!.evaluate((data) => parent.postMessage(data, "*"), payload);
  expect(callbacks).toHaveLength(1);
});

test("landing executes the authenticated Unicorn loader and actual scene under enforcing CSP", async ({ page, context }) => {
  await context.route("https://cdn.jsdelivr.net/gh/hiunicornstudio/unicornstudio.js@v2.1.4/dist/unicornStudio.umd.js", (route) => route.fulfill({
    contentType: "text/javascript", body: fixture("unicornstudio-v2.1.4.js"), headers: { "access-control-allow-origin": "*" },
  }));
  let scenes = 0;
  for (const url of ["https://assets.unicorn.studio/embeds/**", "https://storage.googleapis.com/unicornstudio-production/embeds/**"]) {
    await context.route(url, async (route) => {
      scenes++;
      await route.fulfill({ contentType: "application/json", body: fixture("unicorn-scene.json"), headers: { "access-control-allow-origin": "*" } });
    });
  }
  await page.goto("/");
  await expect(page.locator('[data-us-project] canvas')).toHaveCount(1, { timeout: 15_000 });
  expect(scenes).toBeGreaterThan(0);
  expect(await page.locator('script[src*="unicornstudio.js"]').getAttribute("integrity")).toMatch(/^sha384-/);
});

test("actual Facebook SDK loader and bundle initialize under enforcing CSP", async ({ page, context }) => {
  let bundleRequests = 0;
  await context.route("https://connect.facebook.net/en_US/sdk.js", (route) => route.fulfill({
    contentType: "text/javascript", body: fixture("facebook-sdk.js"), headers: { "access-control-allow-origin": "*" },
  }));
  await context.route("https://connect.facebook.net/en_US/bundle/sdk.js/", async (route) => {
    bundleRequests++;
    await route.fulfill({ contentType: "text/javascript", body: fixture("facebook-sdk-bundle.js"), headers: { "access-control-allow-origin": "*" } });
  });
  await context.route("https://www.facebook.com/**", (route) => route.fulfill({ contentType: "text/html", body: "<!doctype html><title>Meta transport</title>" }));
  await page.goto("/login");
  await page.evaluate(() => new Promise<void>((resolve, reject) => {
    const script = document.createElement("script");
    script.src = "https://connect.facebook.net/en_US/sdk.js";
    script.crossOrigin = "anonymous";
    script.onload = () => resolve(); script.onerror = () => reject(new Error("Facebook SDK loader rejected"));
    document.head.append(script);
  }));
  await expect.poll(() => bundleRequests).toBe(1);
  await expect.poll(() => page.evaluate(() => !!(window as unknown as { FB?: { __buffer?: unknown } }).FB
    && !(window as unknown as { FB?: { __buffer?: unknown } }).FB?.__buffer)).toBe(true);
  await page.evaluate(() => window.FB!.init({ appId: "111", version: "v25.0", cookie: false, xfbml: false }));
  expect(await page.locator('script[src^="https://connect.facebook.net/"]').count()).toBeGreaterThanOrEqual(2);
});

test("claim initial documents, resource referrers and access logs hide normalized paths and encoded bearer values", async ({ browser, request, baseURL }) => {
  const log = process.env.NYXID_NGINX_ACCESS_LOG;
  if (!log) throw new Error("NYXID_NGINX_ACCESS_LOG must name the actual nginx access log");
  // Disable application JavaScript: these guarantees must hold before boot.
  const context = await browser.newContext({ javaScriptEnabled: false });
  const page = await context.newPage();
  const referrers: (string | undefined)[] = [];
  const resourceReferrers: (string | undefined)[] = [];
  await context.route("**/build-version.json?initial-resource-probe", async (route) => {
    resourceReferrers.push(route.request().headers().referer);
    await route.fulfill({ contentType: "image/svg+xml", body: '<svg xmlns="http://www.w3.org/2000/svg"/>' });
  });
  await context.route("https://api.nyxid.example/referrer-probe", async (route) => {
    referrers.push(route.request().headers().referer);
    await route.fulfill({ contentType: "text/html", body: "<title>Referrer probe</title>" });
  });
  const canary = `CLAIM_POLICY_CANARY_${Date.now()}`;
  try {
    const cases = [
      `/channel-bots?claim=${canary}`, `/channel-bots?%63laim=${canary}`,
      `/channel-bots?claim=${canary}&%63laim=${canary}_duplicate`,
      `/channel-bots/?%63laim=${canary}`, `/%63hannel-bots?%63laim=${canary}`,
    ];
    for (const receiver of cases) {
      const response = await page.goto(`${baseURL}${receiver}`);
      expect(response!.headers()["referrer-policy"]).toBe("no-referrer");
      await request.get("/build-version.json", { headers: { referer: `${baseURL}${receiver}` } });
      await page.evaluate(() => {
        const image = document.createElement("img"); image.src = "/build-version.json?initial-resource-probe";
        document.body.append(image);
        const link = document.createElement("a"); link.id = "referrer-probe";
        link.href = "https://api.nyxid.example/referrer-probe"; link.textContent = "Probe";
        document.body.append(link);
      });
      await expect.poll(() => resourceReferrers.length).toBe(referrers.length + 1);
      await page.locator("#referrer-probe").click();
      await expect(page).toHaveURL("https://api.nyxid.example/referrer-probe");
    }
    expect(referrers).toEqual(cases.map(() => undefined));
    expect(resourceReferrers).toEqual(cases.map(() => undefined));
    await expect.poll(() => fs.readFileSync(log, "utf8").includes('"GET /channel-bots HTTP/1.1"')).toBe(true);
    const contents = fs.readFileSync(log, "utf8");
    expect(contents).not.toContain(canary);
    expect(contents).not.toContain("%63laim=");
  } finally { await context.close(); }
});
