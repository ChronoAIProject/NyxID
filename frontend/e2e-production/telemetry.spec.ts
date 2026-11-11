import { expect, type Page } from "@playwright/test";
import { test } from "../e2e/production-policy-fixture";
import { mockDashboard } from "../e2e/managed-onboarding-fixtures";
import { hasOnlyRelayEgress, observeTelemetryEgress } from "../e2e/telemetry-request";

const accountId = "11111111-1111-4111-8111-111111111111";
const conversationId = "nyxa-11111111111141118111111111111111";
const messageId = "22222222-2222-4222-8222-222222222222";
const date = "2026-10-19T00:00:00Z";
const consentKey = `nyxid.telemetry_consent.v3.${accountId}`;
const event = { name: "ui.assistant_feedback_submitted", props: { message_id: messageId, vote: "up" } };

/** Backend transport only. The real built chat, auth and analytics code executes. */
async function feedbackFixture(page: Page) {
  await mockDashboard(page);
  await page.route("**/api/v1/users/me", (route) => route.fulfill({ json: {
    id: accountId, email: "policy@example.test", display_name: "Policy fixture",
    is_admin: false, is_active: true, email_verified: true, created_at: date,
    capabilities: { enabled_features: ["experimental:ai-assistant", "assistant:nyxagent-engine"] },
  } }));
  const agent = { id: "policy-agent", kind: "nyxbot", name: "NyxBot", status: "idle",
    home_conversation_id: conversationId, created_at: date, last_active_at: date };
  const conversation = { id: conversationId, title: "Policy feedback", model: "policy-model",
    created_at: date, last_message_at: date, message_count: 1, active_turn: null, context_reset_at: null,
    agent: { id: agent.id, kind: agent.kind, name: agent.name } };
  const limits = { skip_destructive_confirmation: false, max_live_subagents: 4,
    max_concurrent_subagent_turns: 2, max_live_subagents_limit: 8, max_concurrent_subagent_turns_limit: 4 };
  const feedback: unknown[] = [];
  await page.route("**/api/v1/assistant/nyxagent/**", (route) => {
    const url = new URL(route.request().url());
    const endpoint = url.pathname.replace("/api/v1/assistant/nyxagent", "");
    if (endpoint.endsWith("/feedback")) {
      feedback.push(route.request().postDataJSON());
      return route.fulfill({ json: { vote: route.request().postDataJSON().vote } });
    }
    if (endpoint === "/agents") return route.fulfill({ json: { agents: [agent], limits } });
    if (endpoint === `/agents/${agent.id}`) return route.fulfill({ json: { agent, threads: [conversation] } });
    if (endpoint === "/settings") return route.fulfill({ json: limits });
    if (endpoint === "/models") return route.fulfill({ json: [{ id: "policy-model", label: "Policy model", available: true }] });
    if (endpoint === "/channels") return route.fulfill({ json: { channel_agents: [] } });
    if (endpoint === "/groups") return route.fulfill({ json: { groups: [] } });
    if (endpoint === "/conversations") return route.fulfill({ json: { conversations: [conversation], next_cursor: null } });
    if (endpoint === `/conversations/${conversationId}`) return route.fulfill({ json: {
      conversation, before_seq: null, messages: [{ id: messageId, seq: 1, turn_id: "policy-turn",
        role: "assistant", text: "Policy feedback reply", status: "completed", error_code: null, created_at: date }],
    } });
    if (endpoint === "/live") return route.fulfill({ contentType: "text/event-stream", body: ": fixture\n\n" });
    return route.fulfill({ json: [] });
  });
  return feedback;
}

const cases = [
  { name: "consented US destination", device: true, account: true, host: "https://us.i.posthog.com" },
  { name: "consented EU destination", device: true, account: true, host: "https://eu.i.posthog.com" },
  { name: "consented custom destination", device: true, account: true, host: "https://analytics.example" },
  { name: "device declined", device: false, account: true },
  { name: "account declined", device: true, account: false },
  { name: "Do Not Track", device: true, account: true, dnt: true },
  { name: "another account's device choice", device: true, account: true, otherAccount: true },
  { name: "legacy unbound opt-in", device: true, account: true, legacy: true },
] as const;

for (const scenario of cases) {
  test(`production same-origin analytics relay: ${scenario.name}`, async ({ page, baseURL, context }) => {
    const host = "host" in scenario ? scenario.host : "https://us.i.posthog.com";
    const shouldSend = scenario.device && scenario.account && !("dnt" in scenario || "otherAccount" in scenario || "legacy" in scenario);
    await page.addInitScript(({ key, scenario }) => {
      const choice = { enabled: scenario.device, asked: true };
      if ("legacy" in scenario) {
        localStorage.setItem("nyxid.telemetry_consent", JSON.stringify({ state: choice, version: 1 }));
      } else {
        localStorage.setItem("otherAccount" in scenario ? `${key}-other-account` : key, JSON.stringify(choice));
      }
      if ("dnt" in scenario) Object.defineProperty(navigator, "doNotTrack", { get: () => "1" });
    }, { key: consentKey, scenario });
    await context.addCookies([{ name: "nyx_session", value: "policy-fixture-session", url: baseURL!, httpOnly: true, sameSite: "Lax" }]);
    const feedback = await feedbackFixture(page);
    await page.route("**/api/v1/public/config", (route) => route.fulfill({ json: {
      telemetry_dsn: "policy-fixture-configured", telemetry_host: host, telemetry_share_analytics: false,
    } }));
    let accountEnabled: boolean = scenario.account;
    const admissions: string[] = [];
    const deliveries: unknown[] = [];
    await page.route("**/api/v1/users/me/preferences/analytics", (route) => {
      const revision = `policy-revision-${admissions.length + 1}`;
      admissions.push(revision);
      return route.fulfill({ json: { account_id: accountId, enabled: accountEnabled, revision, changed_at: date } });
    });
    await page.route("**/api/v1/users/me/analytics/events", (route) => {
      expect(route.request().method()).toBe("POST");
      expect(route.request().headers()["cookie"]).toContain("nyx_session=policy-fixture-session");
      expect(route.request().headers()["x-nyxid-analytics-account"]).toBe(accountId);
      deliveries.push(route.request().postDataJSON());
      return route.fulfill({ status: 204 });
    });
    const requests: string[] = [];
    const appOrigin = new URL(baseURL!).origin;
    page.on("request", (request) => observeTelemetryEgress(requests, request.url(), request.resourceType(), appOrigin));
    await page.goto(`/assistant?c=${conversationId}`);
    await expect(page.getByText("Policy feedback reply", { exact: true })).toBeVisible();
    // No pageview/autocapture or anonymous startup delivery.
    expect(deliveries).toEqual([]);
    await page.getByRole("button", { name: "Helpful", exact: true }).click();
    await expect.poll(() => feedback.length).toBe(1);
    if (shouldSend) {
      await expect.poll(() => deliveries.length).toBe(1);
      expect(deliveries).toEqual([{ account_id: accountId, revision: admissions[0], event }]);
      expect(admissions).toHaveLength(1);
      // A fresh read must observe withdrawal rather than reusing the positive.
      accountEnabled = false;
      await page.getByRole("button", { name: "Not helpful", exact: true }).click();
      await expect.poll(() => admissions.length).toBe(2);
    } else if (scenario.device && !scenario.account) {
      await expect.poll(() => admissions.length).toBe(1);
    }
    await page.waitForTimeout(700);
    expect(deliveries).toHaveLength(shouldSend ? 1 : 0);
    if (!shouldSend && scenario.account) expect(admissions).toEqual([]);
    expect(hasOnlyRelayEgress(requests, appOrigin), JSON.stringify(requests)).toBe(true);
    expect(await page.locator('script[src*="posthog"], script[src*="analytics.example"]').count()).toBe(0);
  });
}

test("anonymous login ignores legacy opt-in and sends no analytics", async ({ page, baseURL }) => {
  await page.addInitScript(() => localStorage.setItem("nyxid.telemetry_consent", JSON.stringify({ state: { asked: true, enabled: true }, version: 1 })));
  await page.route("**/api/v1/public/config", (route) => route.fulfill({ json: {
    email_auth_enabled: true, social_providers: [], telemetry_dsn: "policy-fixture-configured",
  } }));
  const requests: string[] = [];
  page.on("request", (request) => observeTelemetryEgress(requests, request.url(), request.resourceType(), baseURL!));
  await page.goto("/login");
  await expect(page.getByRole("button", { name: "Sign in", exact: true })).toBeVisible();
  await page.getByLabel("Email", { exact: true }).fill("policy@example.test");
  await page.waitForTimeout(700);
  expect(requests).toEqual([]);
});

for (const origin of ["https://us.i.posthog.com", "https://eu.i.posthog.com", "https://analytics.example"]) {
  test(`enforcing CSP blocks direct analytics ingest at ${origin}`, async ({ page }, testInfo) => {
    const url = `${origin}/i/v0/e/`;
    testInfo.annotations.push({ type: "expected-csp", description: JSON.stringify({ directive: "connect-src", blocked: url }) });
    let attempts = 0;
    await page.route(`${origin}/**`, (route) => { attempts++; return route.fulfill({ status: 204 }); });
    await page.goto("/login");
    expect(await page.evaluate(async (url) => {
      try { await fetch(url, { method: "POST", body: "policy-fixture" }); return false; }
      catch { return true; }
    }, url)).toBe(true);
    expect(attempts).toBe(0);
  });
}
