import { expect, test, type Locator, type Page } from "@playwright/test";
import type { QueryClient } from "@tanstack/react-query";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { homedir } from "node:os";
import { openAssistant, sendMessage, composerInput } from "./helpers";
import {
  cmaFrame,
  cmaHead,
  cmaSession,
} from "../src/lib/assistant/cma-session.test-fixtures";

// Explicit server-receipt fixture; it is fixed to a seeded source thread before
// rendering any CMA cards and never follows later navigation.
const sourceByPage = new WeakMap<
  Page,
  { conversation: string; specialist: string }
>();
async function openCmaAssistant(
  page: Page,
  options: Parameters<typeof openAssistant>[1] & { specialist?: boolean } = {},
) {
  await openAssistant(page, options);
  const source = await page.evaluate((useSpecialist) => {
    const state = JSON.parse(
      sessionStorage.getItem("nyxagent-http-fixture-v4")!,
    ) as {
      rows: [string, { agentId: string }][];
      agents: { id: string; kind: string }[];
    };
    const specialist = state.agents.find(
      (agent) => agent.kind === (useSpecialist ? "specialist" : "nyxbot"),
    )!.id;
    return {
      conversation: state.rows.find(
        ([, row]) => row.agentId === specialist,
      )![0],
      specialist,
    };
  }, Boolean(options.specialist));
  sourceByPage.set(page, source);
  await page.goto(
    `/assistant?mock=1&c=${source.conversation}&agent=${source.specialist}`,
  );
  await expect(composerInput(page)).toBeVisible();
}

async function mockCma(
  page: Page,
  options: {
    actionCount?: number;
    idle?: boolean;
    multiple?: boolean;
    receiptCallback?: string | null;
    headStatus?: number;
    headDelayMs?: number;
    pinDelayMs?: number;
    pinFailure?: number;
    deniedConnection?: boolean;
  } = {},
) {
  let active = !options.idle;
  let completed = false;
  let callback: unknown = undefined;
  let tails = 0;
  let heads = 0;
  const deniedId = "00000000-0000-4000-8000-000000000001";
  const actionCount = options.actionCount ?? 1;
  const requests: string[] = [];
  await page.route(
    "**/api/v1/assistant/nyxagent/cma-sessions?*",
    async (route) => {
      if (
        new URL(route.request().url()).searchParams.has("pins") &&
        options.pinDelayMs
      )
        await new Promise((resolve) => setTimeout(resolve, options.pinDelayMs));
      if (
        new URL(route.request().url()).searchParams.has("pins") &&
        options.pinFailure
      )
        return route.fulfill({
          status: options.pinFailure,
          json: {
            error: "unavailable",
            message: "Temporary outage",
            error_code: 1,
          },
        });
      const source = sourceByPage.get(page);
      return route.fulfill({
        json: {
          sessions: source
            ? [
                cmaSession.agentId,
                ...(options.multiple ? ["agt_second"] : []),
              ].map((agent_id) => ({
                service_id: cmaSession.connection.id,
                agent_id,
                conversation_id: source.conversation,
                specialist_id: source.specialist,
                turn_id: "original-turn",
                connection_name: cmaSession.connection.name,
                connection_slug: cmaSession.connection.slug,
                cma_url:
                  options.receiptCallback === undefined
                    ? "https://cma.example/agents/agt_observed"
                    : options.receiptCallback,
              }))
            : [],
          next_cursor: null,
          connections: source
            ? [
                ...(options.deniedConnection
                  ? [{ ...cmaSession.connection, id: deniedId }]
                  : []),
                cmaSession.connection,
              ]
            : [],
        },
      });
    },
  );
  await page.route("**/api/v1/proxy/**", async (route) => {
    const url = new URL(route.request().url());
    requests.push(url.pathname + url.search);
    expect(route.request().method()).toBe("GET");
    if (url.pathname === "/api/v1/proxy/services") {
      return route.fulfill({
        json: {
          services: [
            cmaSession.connection,
            {
              ...cmaSession.connection,
              id: "32ec5c70-b8bb-4133-8824-031a1dc63945",
              name: "GitHub",
              slug: "github",
            },
          ],
          custom_services: [],
          total: 2,
          page: 1,
          per_page: 100,
        },
      });
    }
    if (url.pathname.includes(deniedId))
      return route.fulfill({
        status: 403,
        json: {
          error: "forbidden",
          message: "Revoked connection",
          error_code: 1003,
        },
      });
    if (url.pathname.endsWith("/openapi.json")) {
      return route.fulfill({
        json: {
          paths: Object.fromEntries(
            Object.values(cmaSession.routes).map((path) => [path, { get: {} }]),
          ),
        },
      });
    }
    const second = url.pathname.includes("/agt_second/");
    if (url.pathname.endsWith("/events/head")) {
      heads++;
      if (options.headDelayMs)
        await new Promise((resolve) =>
          setTimeout(resolve, options.headDelayMs),
        );
      if (options.headStatus)
        return route.fulfill({
          status: options.headStatus,
          json: {
            error: "forbidden",
            message: "Stream unavailable",
            error_code: 1003,
          },
        });
      return route.fulfill({
        json: {
          ...cmaHead,
          agent_id: second ? "agt_second" : cmaSession.agentId,
        },
      });
    }
    if (url.pathname.endsWith("/events")) {
      tails++;
      const after = url.searchParams.get("after_cursor");
      const body =
        (after === "cursor-10"
          ? Array.from({ length: actionCount }, (_, index) =>
              cmaFrame(11 + index, {
                type: "tool_started",
                call_id: index ? `inspect-${index}` : "inspect",
                title: index
                  ? `Checking page section ${index}`
                  : "Inspecting the browser page",
                status: "in_progress",
              }),
            ).join("")
          : "") +
        (completed
          ? cmaFrame(11 + actionCount, {
              type: "tool_updated",
              call_id: "inspect",
              status: "completed",
            }) +
            cmaFrame(12 + actionCount, {
              type: "turn_completed",
              turn_sequence: 10,
            })
          : "");
      return route.fulfill({
        contentType: "text/event-stream",
        body: (body || ": keepalive\n\n").replaceAll(
          cmaSession.agentId,
          second ? "agt_second" : cmaSession.agentId,
        ),
      });
    }
    const agent = {
      id: cmaSession.agentId,
      title: cmaSession.title,
      status: active ? "working" : "ready",
      ...(callback === undefined ? {} : { callback_url: callback }),
    };
    const otherAgent = {
      id: "agt_second",
      title: "Check deployment",
      status: "waiting_approval",
    };
    return route.fulfill({
      json: url.pathname.endsWith("/agents")
        ? {
            items: [agent, ...(options.multiple ? [otherAgent] : [])],
            next: null,
          }
        : url.pathname.endsWith("/agt_second")
          ? otherAgent
          : agent,
    });
  });
  return {
    requests,
    setCallback: (value: unknown) => {
      callback = value;
    },
    start: () => {
      active = true;
    },
    finishStream: () => {
      completed = true;
    },
    markIdle: () => {
      active = false;
    },
    tailCount: () => tails,
    headCount: () => heads,
  };
}

const sessions = (page: Page) =>
  page.getByRole("complementary", { name: "CMA sessions" });
const card = (page: Page) =>
  sessions(page).getByRole("region", {
    name: `CMA session: ${cmaSession.title}`,
    exact: true,
  });

async function historyBounds(panel: Locator, label: string) {
  const button = panel.getByRole("button", { name: label, exact: true });
  await button.scrollIntoViewIfNeeded();
  await expect(button).toBeVisible();
  const bounds = await button.evaluate(button => {
    const header = button.parentElement!;
    const body = button.closest('[aria-label^="CMA session:"]')!;
    const box = button.getBoundingClientRect(), outer = body.getBoundingClientRect();
    const range = document.createRange();
    range.selectNodeContents(button);
    return {
      button: { client: button.clientWidth, scroll: button.scrollWidth },
      header: { client: header.clientWidth, scroll: header.scrollWidth },
      body: { left: outer.left, right: outer.right },
      box: { left: box.left, right: box.right, top: box.top, bottom: box.bottom },
      text: [...range.getClientRects()].map(rect => ({ left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom })),
    };
  });
  expect(bounds.button.scroll, label).toBeLessThanOrEqual(bounds.button.client + 1);
  expect(bounds.header.scroll, label).toBeLessThanOrEqual(bounds.header.client + 1);
  expect(bounds.box.left).toBeGreaterThanOrEqual(bounds.body.left);
  expect(bounds.box.right).toBeLessThanOrEqual(bounds.body.right);
  for (const text of bounds.text) {
    expect(text.left).toBeGreaterThanOrEqual(bounds.box.left - 1);
    expect(text.right).toBeLessThanOrEqual(bounds.box.right + 1);
    expect(text.top).toBeGreaterThanOrEqual(bounds.box.top - 1);
    expect(text.bottom).toBeLessThanOrEqual(bounds.box.bottom + 1);
  }
  return bounds;
}

for (const theme of ["light", "dark"] as const) for (const width of [1440, 390]) for (const density of [1, 1.4]) for (const actionCount of [5, 30]) {
  test(`CMA history labels fit source and floating cards (${theme}, ${width}, spacing ${density}×, ${actionCount} actions)`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1000 });
    await page.addInitScript(({ theme, density }) => localStorage.setItem("nyxid.theme", JSON.stringify({
      state: { mode: theme, textScale: 2, density, motion: "reduce" }, version: 2,
    })), { theme, density });
    await mockCma(page, { actionCount });
    await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
    const panel = card(page);
    const measurements = [];
    for (const shell of ["assistant", "studio"]) {
      const collapsed = await historyBounds(panel, `Show all ${actionCount} actions`);
      await panel.getByRole("button", { name: `Show all ${actionCount} actions`, exact: true }).click();
      const expanded = await historyBounds(panel, "Show recent actions");
      measurements.push({ shell, collapsed, expanded });
      if (process.env.NYX_REVIEW_SHOTS && density === 1.4 && actionCount === 30) {
        await mkdir(process.env.NYX_REVIEW_SHOTS, { recursive: true });
        await page.screenshot({ animations: "disabled", path: join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-cma-${shell}.png`) });
      }
      await panel.getByRole("button", { name: "Show recent actions", exact: true }).click();
      if (shell === "assistant") {
        await panel.getByRole("button", { name: "Pin CMA session", exact: true }).click();
        await page.goto("/keys?mock=1");
        await expect(panel.getByRole("button", { name: "Unpin CMA session (Pinned)", exact: true })).toBeVisible();
      }
    }
    const metrics = JSON.stringify(measurements, null, 2);
    await test.info().attach("cma-history-bounds.json", { body: metrics, contentType: "application/json" });
    if (process.env.NYX_REVIEW_SHOTS) {
      await mkdir(process.env.NYX_REVIEW_SHOTS, { recursive: true });
      await writeFile(join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${density}-${actionCount}-cma-history.json`), metrics);
    }
  });
}

for (const theme of ["light", "dark"] as const) {
  for (const width of [1440, 390]) {
    for (const scale of [0.5, 2]) {
      test(`CMA roles and controls retain shared treatment across shells (${theme}, ${width}, ${scale}×)`, async ({ page }) => {
        await page.setViewportSize({ width, height: 1000 });
        await page.addInitScript(({ theme, scale }) => {
          localStorage.setItem("nyxid.theme", JSON.stringify({
            state: { mode: theme, textScale: scale, density: 1, motion: "reduce" },
            version: 2,
          }));
        }, { theme, scale });
        await mockCma(page, { actionCount: 5 });
        await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
        const panel = card(page);
        const history = panel.getByRole("button", { name: "Show all 5 actions" });
        await expect(history).toBeVisible();
        await expect(panel.getByRole("heading", { name: cmaSession.title, exact: true }))
          .toHaveCSS("font-size", `${15 * 0.85 * scale}px`);
        await expect(panel.getByRole("heading", { name: cmaSession.title, exact: true }))
          .toHaveCSS("font-weight", "600");
        await expect(history).toHaveCSS("font-size", `${14 * 0.85 * scale}px`);
        await expect(history).toHaveCSS("font-weight", "500");
        const indicator = page.getByLabel("CMA is working in this session", { exact: true });
        await expect(indicator).toHaveCSS("font-size", `${12 * 0.85 * scale}px`);
        expect(await indicator.evaluate(element =>
          element.getBoundingClientRect().height >= parseFloat(getComputedStyle(element).lineHeight),
        )).toBe(true);

        const controls = panel.getByRole("group", { name: "CMA session controls" });
        const before = await controls.locator("[data-icon-button]").evaluateAll(elements => elements.map(element => {
          const style = getComputedStyle(element);
          const icon = getComputedStyle(element.querySelector("svg")!);
          return {
            label: element.getAttribute("aria-label"),
            box: { width: style.width, height: style.height, padding: style.padding, gap: style.gap, radius: style.borderRadius, fontSize: style.fontSize, lineHeight: style.lineHeight, icon: { width: icon.width, height: icon.height } },
          };
        }));
        expect(before).toHaveLength(4);
        for (const control of before) {
          expect(control.box).toEqual(before[0]!.box);
          expect(parseFloat(control.box.width)).toBeGreaterThanOrEqual(24);
          expect(parseFloat(control.box.height)).toBeGreaterThanOrEqual(24);
        }
        await panel.getByRole("button", { name: "Pin CMA session" }).click();
        await page.goto("/keys?mock=1");
        await expect(panel.getByRole("button", { name: "Unpin CMA session (Pinned)" })).toBeVisible();
        for (const { label, box } of before.filter(control => control.label !== "Pin CMA session")) {
          const control = controls.getByLabel(label!, { exact: true });
          const after = await control.evaluate(element => {
            const style = getComputedStyle(element);
            const icon = getComputedStyle(element.querySelector("svg")!);
            return { width: style.width, height: style.height, padding: style.padding, gap: style.gap, radius: style.borderRadius, fontSize: style.fontSize, lineHeight: style.lineHeight, icon: { width: icon.width, height: icon.height } };
          });
          expect(after).toEqual(box);
        }
      });
    }
  }
}


for (const width of [1440, 390]) {
  test(`shows an unconfirmed CMA submission in the shared assets area (${width}px)`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await mockCma(page, { idle: true });
    await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
    await page.evaluate(() => {
      const state = JSON.parse(
        sessionStorage.getItem("nyxagent-http-fixture-v4")!,
      ) as {
        rows: [string, { history: { cma_operations: unknown[] } }][];
      };
      const conversation = new URL(location.href).searchParams.get("c");
      const row = state.rows.find(([id]) => id === conversation)![1];
      row.history.cma_operations = [
        {
          id: "unconfirmed-submission",
          turn_id: "older-unloaded-turn",
          service_id: "22ec5c70-b8bb-4133-8824-031a1dc63945",
          kind: "cma_provisioning",
          operation_id: null,
          agent_id: null,
          idempotency_key: "cma-setup-preview-20261110-worker2",
          status: null,
          phase: null,
          state: "delivered",
          reason: "submission_uncertain",
          monitor_name: "NyxBot",
          created_at: "2026-11-10T10:00:00Z",
          checked_at: null,
        },
      ];
      sessionStorage.setItem("nyxagent-http-fixture-v4", JSON.stringify(state));
    });
    await page.reload();
    const receipt = page.getByRole("region", {
      name: "CMA submission: CMA setup",
      exact: true,
    });
    await expect(receipt.getByRole("status")).toHaveText(
      "Submission unconfirmed",
    );
    await expect(
      receipt.getByRole("heading", { name: "CMA setup" }),
    ).toBeVisible();
    await expect(receipt.getByText("Delegated to CMA")).toHaveCount(0);
    await expect(receipt.getByText("Recent actions")).toHaveCount(0);
    await expect(receipt.getByRole("button", { name: /Pin CMA/ })).toHaveCount(
      0,
    );
    await receipt.getByRole("button", { name: "Submission details" }).click();
    await expect(
      receipt.getByText("cma-setup-preview-20261110-worker2"),
    ).toBeVisible();
    await receipt.getByRole("button", { name: "Submission details" }).click();
    await receipt
      .getByRole("button", { name: "Minimise CMA submission" })
      .click();
    await expect(
      receipt.getByRole("button", { name: "Submission details" }),
    ).toHaveCount(0);
    await expect(receipt.getByRole("status")).toHaveText(
      "Submission unconfirmed",
    );
    await receipt
      .getByRole("button", { name: "Maximise CMA submission" })
      .click();
    await expect(page.locator("article [data-cma-operation]")).toHaveCount(0);
    const preview = page.getByRole("complementary", {
      name: "Preview",
      exact: true,
    });
    if (width === 1440) {
      await expect(
        preview.getByRole("region", { name: "CMA submission: CMA setup" }),
      ).toBeVisible();
      const bounds = (await preview.boundingBox())!;
      const composer = (await composerInput(page).boundingBox())!;
      expect(composer.x + composer.width).toBeLessThanOrEqual(bounds.x);
    } else {
      await expect(preview).toHaveCount(0);
      await expect(receipt).toBeVisible();
      await expect(composerInput(page)).toBeVisible();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    }
    await page.screenshot({
      path: `/tmp/nyxid-cma-submission-assets-${width}.png`,
    });
    await page.reload();
    await expect(receipt).toBeVisible();
    const sourceUrl = page.url();
    for (const path of ["/assistant?mock=1", "/keys?mock=1"]) {
      await page.goto(path);
      await expect(receipt).toHaveCount(0);
    }
    await page.goto(sourceUrl);
    await expect(receipt).toBeVisible();
    await expect(receipt.getByRole("button", { name: /Pin CMA/ })).toHaveCount(0);
  });
}

for (const width of [1440, 390]) {
  test(`retains request status and one execution entry in the original live card (${width}px)`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await mockCma(page, { actionCount: 5 });
    await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
    await page.evaluate(({ serviceId, agentId }) => {
      const state = JSON.parse(sessionStorage.getItem("nyxagent-http-fixture-v4")!) as {
        rows: [string, { history: { cma_operations: unknown[] } }][];
      };
      const conversation = new URL(location.href).searchParams.get("c");
      const row = state.rows.find(([id]) => id === conversation)![1];
      row.history.cma_operations = [0, 1].map((index) => ({
        id: `linked-request-${index}`, turn_id: "older-unloaded-turn",
        service_id: serviceId, agent_id: agentId,
        agent_ref_id: "0dc85e4c-6aaf-47de-9df0-e07a69b8ee19",
        response_id: `rsp-linked-${index}`, operation_id: `rsp-linked-${index}`,
        kind: "cma_response", idempotency_key: `preview-request-${index}`,
        status: index ? "in_progress" : "queued", phase: null,
        state: "waiting", reason: null, monitor_name: "NyxBot",
        created_at: new Date(Date.now() - (index ? 12 : 1) * 60_000).toISOString(),
        checked_at: new Date().toISOString(),
      }));
      sessionStorage.setItem("nyxagent-http-fixture-v4", JSON.stringify(state));
    }, { serviceId: cmaSession.connection.id, agentId: cmaSession.agentId });
    await page.reload();
    const live = card(page);
    await expect(live).toBeVisible();
    await page.evaluate(async () => {
      const { useAuthStore } = await import("/src/stores/auth-store.ts");
      const user = useAuthStore.getState().user!;
      useAuthStore.getState().setUser({ ...user, capabilities: {
        ...user.capabilities,
        billing_available: user.capabilities?.billing_available ?? false,
        enabled_features: [...(user.capabilities?.enabled_features ?? []), "assistant:cma-sandboxes"],
      } });
    });
    await expect(page.locator("[data-cma-operation]")).toHaveCount(0);
    const submissionDetails = live.getByRole("button", { name: "Submission details", exact: true });
    await expect(submissionDetails).toHaveCount(1);
    await expect(submissionDetails).toHaveAttribute("aria-expanded", "false");
    await expect(live.getByRole("region", { name: "CMA requests", exact: true })).toHaveCount(0);
    await submissionDetails.click();
    const requests = live.getByRole("region", { name: "CMA requests", exact: true });
    const activity = live.getByRole("region", { name: "CMA activity", exact: true });
    await expect(live.locator(".animate-spin")).toHaveCount(1);
    await expect(activity.getByRole("listitem").first().locator(".animate-spin")).toHaveCount(1);
    await expect(requests.locator(".animate-spin")).toHaveCount(0);
    await live.getByRole("button", { name: "Show all 5 actions", exact: true }).click();
    await expect(activity.getByRole("listitem")).toHaveCount(5);
    await expect(live.locator(".animate-spin")).toHaveCount(1);
    await live.getByRole("button", { name: "Show recent actions", exact: true }).click();
    await expect(requests.locator('[data-cma-request="linked-request-0"]').getByRole("status")).toHaveText("Queued");
    await expect(requests.locator('[data-cma-request="linked-request-1"]').getByRole("status")).toHaveText("Taking longer than usual");
    await expect(live.getByRole("status", { name: "CMA agent status: Working" })).toHaveText("Working");
    await expect(requests.getByRole("heading", { name: "Request status" })).toBeVisible();
    const controls = live.getByRole("group", { name: "CMA session controls" });
    const execution = controls.getByRole("button", { name: "View execution", exact: true });
    await expect(execution).toHaveCount(1);
    await expect(execution).toHaveText("");
    await expect(requests.getByRole("button", { name: "View execution" })).toHaveCount(0);
    await execution.focus();
    await expect(page.getByRole("tooltip", { name: "View execution", exact: true })).toBeVisible();
    await execution.blur();
    await live.getByRole("button", { name: "Minimise CMA session", exact: true }).click();
    await expect(execution).toBeVisible();
    await expect(requests).toHaveCount(0);
    await expect(live.locator(".animate-spin")).toHaveCount(1);
    await expect(live.getByRole("status", { name: "CMA agent status: Working" }).locator(".animate-spin")).toHaveCount(1);
    await live.getByRole("button", { name: "Maximise CMA session", exact: true }).click();
    await expect(activity.getByRole("listitem").first().locator(".animate-spin")).toHaveCount(1);
    await live.getByRole("button", { name: "Pin CMA session", exact: true }).click();
    await expect(live.getByRole("button", { name: "Unpin CMA session (Pinned)" })).toHaveAttribute("aria-pressed", "true");
    await expect(requests.getByText("rsp-linked-0", { exact: true })).toBeVisible();
    await requests.getByText("rsp-linked-1", { exact: true }).scrollIntoViewIfNeeded();
    await expect(requests.getByText("rsp-linked-1", { exact: true })).toBeVisible();
    await expect(requests.getByRole("button", { name: "Submission details" })).toHaveCount(0);
    await expect(submissionDetails).toHaveCount(1);
    await expect(execution).toBeVisible();
    await expect(composerInput(page)).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await live.locator(":scope > div").last().evaluate((body) => { body.scrollTop = 0; });
    await page.screenshot({ path: `/tmp/nyxid-cma-submissions-expanded-${width}.png` });
    await live.screenshot({ path: `/tmp/nyxid-cma-submissions-expanded-card-${width}.png` });
    await submissionDetails.click();
    await expect(requests).toHaveCount(0);
    await expect(submissionDetails).toHaveAttribute("aria-expanded", "false");
    await expect(execution).toBeVisible();
    await expect(live.locator(".animate-spin")).toHaveCount(1);
    await live.locator(":scope > div").last().evaluate((body) => { body.scrollTop = 0; });
    await page.screenshot({ path: `/tmp/nyxid-cma-submissions-${width}.png` });
    await live.screenshot({ path: `/tmp/nyxid-cma-submissions-card-${width}.png` });
  });
}

test("automatically shows running CMA, streams while minimised, restores, and removes finished sessions", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const cma = await mockCma(page);
  await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
  const panel = card(page);
  await expect(panel.getByText("Inspecting the browser page")).toBeVisible();
  await expect(
    sessions(page).getByRole("heading", {
      name: cmaSession.title,
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    panel.getByRole("link", { name: "Open source chat" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Pin session", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "CMA session", exact: true }),
  ).toHaveCount(0);
  const main = await page.getByRole("main").boundingBox();
  const bounds = await panel.boundingBox();
  const preview = (await page
    .getByRole("complementary", { name: "Preview", exact: true })
    .boundingBox())!;
  expect(bounds!.x).toBeCloseTo(preview.x + 12, 0);
  expect(bounds!.width).toBeCloseTo(Math.min(500, preview.width - 12), 0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(
    preview.x + preview.width,
  );
  expect(
    (await composerInput(page).boundingBox())!.x +
      (await composerInput(page).boundingBox())!.width,
  ).toBeLessThanOrEqual(preview.x);
  expect(bounds!.y).toBeGreaterThanOrEqual(main!.y);
  expect(bounds!.height).toBeLessThan(480);
  expect(bounds!.y + bounds!.height).toBeLessThan(main!.y + main!.height - 100);
  await sendMessage(page, "List connected services");
  await expect(
    page.getByRole("link", { name: "Connect GitHub" }),
  ).toBeVisible();
  await page.screenshot({ path: "/tmp/nyxid-cma-auto-desktop.png" });
  await panel.getByRole("button", { name: "Minimise CMA session" }).click();
  await expect(
    panel.getByRole("region", { name: "CMA activity" }),
  ).not.toBeVisible();
  const minimised = await panel.boundingBox();
  expect(minimised!.height).toBeLessThan(bounds!.height);
  await page.screenshot({ path: "/tmp/nyxid-cma-auto-minimised.png" });
  await panel.getByRole("button", { name: "Pin CMA session" }).click();
  await page
    .getByRole("button", { name: "New chat with NyxBot", exact: true })
    .click();
  await expect(composerInput(page)).toBeVisible();
  await expect(
    panel.getByRole("button", { name: "Maximise CMA session" }),
  ).toBeVisible();
  await page.reload();
  await expect(panel.getByRole("status")).toHaveText("Working");
  await expect(
    panel.getByRole("region", { name: "CMA activity" }),
  ).not.toBeVisible();
  const tailsBefore = cma.tailCount();
  cma.finishStream();
  await expect(panel.getByRole("status")).toHaveText("Completed");
  expect(cma.tailCount()).toBeGreaterThan(tailsBefore);
  await panel.getByRole("button", { name: "Maximise CMA session" }).click();
  await expect(panel.getByLabel("completed", { exact: true })).toBeVisible();
  cma.markIdle();
  await expect(sessions(page)).toHaveCount(0, { timeout: 12_000 });
  await expect(composerInput(page)).toBeVisible();
  expect(cma.requests.some((path) => path.includes("32ec5c70"))).toBe(false);
});

test("discovers a session that starts after opening chat without any manual action", async ({
  page,
}) => {
  const cma = await mockCma(page, { idle: true });
  await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
  await expect
    .poll(
      () =>
        cma.requests.filter(
          (url) =>
            url.includes("/api/v1/agents?status=") ||
            /\/api\/v1\/agents\?.*status=/.test(url),
        ).length,
    )
    .toBeGreaterThan(0);
  await expect(sessions(page)).toHaveCount(0);
  expect(cma.tailCount()).toBe(0);
  cma.start();
  await expect(card(page).getByText("Inspecting the browser page")).toBeVisible(
    { timeout: 22_000 },
  );
});

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 390, height: 844 },
]) {
  test(`opens session details within a full activity card at ${viewport.width}px`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await mockCma(page, { actionCount: 30 });
    await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
    const panel = card(page);
    await expect(
      panel.getByRole("region", { name: "CMA activity" }).getByRole("listitem"),
    ).toHaveCount(3);
    const history = panel.getByRole("button", { name: "Show all 30 actions" });
    await expect(history).toHaveAttribute("aria-expanded", "false");
    await history.click();
    await expect(
      panel.getByRole("region", { name: "CMA activity" }).getByRole("listitem"),
    ).toHaveCount(30);
    const details = panel.getByRole("button", {
      name: "Session details",
      exact: true,
    });
    const connection = panel.getByText(cmaSession.connection.name, {
      exact: true,
    });
    const sessionId = panel.getByText(cmaSession.agentId, { exact: true });
    await expect(details).toHaveAttribute("aria-expanded", "false");
    await expect(sessionId).not.toBeVisible();
    const composerBefore = await composerInput(page).boundingBox();
    await details.click();
    await expect(details).toHaveAttribute("aria-expanded", "true");
    await expect(connection).toBeInViewport({ ratio: 1 });
    await expect(sessionId).toBeInViewport({ ratio: 1 });
    await expect(details).toBeInViewport({ ratio: 1 });
    const expandedBounds = await panel.boundingBox();
    const idBounds = await sessionId.boundingBox();
    expect(idBounds!.y + idBounds!.height).toBeLessThanOrEqual(
      expandedBounds!.y + expandedBounds!.height,
    );
    expect((await composerInput(page).boundingBox())!.y).toBe(
      composerBefore!.y,
    );
    await details.press("Space");
    await expect(details).toHaveAttribute("aria-expanded", "false");
    await expect(sessionId).not.toBeVisible();
    await details.press("Enter");
    await expect(sessionId).toBeInViewport({ ratio: 1 });
    await panel.getByRole("button", { name: "Minimise CMA session" }).click();
    await expect(details).not.toBeVisible();
    await panel.getByRole("button", { name: "Maximise CMA session" }).click();
    await expect(details).toHaveAttribute("aria-expanded", "true");
    await expect(sessionId).toBeInViewport({ ratio: 1 });
    await page.screenshot({
      path: `/tmp/nyxid-cma-details-${viewport.width}.png`,
    });
  });
}

test("keeps the automatic CMA card and chat usable together on a narrow screen", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await mockCma(page);
  await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
  const panel = card(page);
  await expect(panel.getByText("Inspecting the browser page")).toBeVisible();
  await expect(composerInput(page)).toBeVisible();
  const bounds = await panel.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
  expect(bounds!.height).toBeLessThanOrEqual(844 * 0.4);
  const composer = await composerInput(page).boundingBox();
  expect(composer!.y).toBeGreaterThan(bounds!.y + bounds!.height);
  expect(composer!.y + composer!.height).toBeLessThanOrEqual(844);
  await sendMessage(page, "List connected services");
  await expect(
    page.getByRole("link", { name: "Connect GitHub" }),
  ).toBeVisible();
  await page.screenshot({ path: "/tmp/nyxid-cma-auto-mobile.png" });
  await panel.getByRole("button", { name: "Minimise CMA session" }).click();
  await expect(panel.getByRole("status")).toHaveText("Working");
  expect((await panel.boundingBox())!.height).toBeLessThan(bounds!.height);
  await panel.getByRole("button", { name: "Maximise CMA session" }).click();
  await expect(panel.getByText("Inspecting the browser page")).toBeVisible();
});

test("bounds long activity and keeps minimise reachable with the mobile keyboard", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mockCma(page, { actionCount: 30 });
  await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
  const panel = card(page);
  const actions = panel.getByRole("region", { name: "CMA activity" });
  await expect(actions.getByRole("listitem")).toHaveCount(3);
  await panel.getByRole("button", { name: "Show all 30 actions" }).click();
  await expect(actions.getByRole("listitem")).toHaveCount(30);
  await expect(actions.getByRole("listitem").first()).toHaveText(
    "Checking page section 29",
  );
  expect((await panel.boundingBox())!.height).toBeLessThanOrEqual(480);
  await expect(
    panel.getByRole("button", { name: "Minimise CMA session" }),
  ).toBeInViewport();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(composerInput(page)).toBeInViewport();
  await page.evaluate(() => {
    Object.defineProperties(window.visualViewport!, {
      height: { configurable: true, value: 420 },
      offsetTop: { configurable: true, value: 60 },
    });
    window.visualViewport!.dispatchEvent(new Event("resize"));
  });
  const composer = await composerInput(page).boundingBox();
  expect(composer!.y + composer!.height).toBeLessThanOrEqual(480);
  await expect(
    panel.getByRole("button", { name: "Minimise CMA session" }),
  ).toBeInViewport();
  await panel.getByRole("button", { name: "Minimise CMA session" }).click();
  await expect(panel.getByRole("status")).toHaveText("Working");
});

test("shows simultaneous CMA sessions and minimises each independently", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mockCma(page, { multiple: true });
  await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
  const first = card(page);
  const second = sessions(page).getByRole("region", {
    name: "CMA session: Check deployment",
    exact: true,
  });
  await expect(first.getByText("Inspecting the browser page")).toBeVisible();
  await expect(second.getByText("Inspecting the browser page")).toBeVisible();
  await first.getByRole("button", { name: "Minimise CMA session" }).click();
  await expect(
    first.getByRole("region", { name: "CMA activity" }),
  ).not.toBeVisible();
  await expect(
    second.getByRole("region", { name: "CMA activity" }),
  ).toBeVisible();
  await expect(composerInput(page)).toBeVisible();
});

test("stacks CMA and machine cards in one resizable preview column", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mockCma(page);
  await page.route("**/api/v1/assistant/nyxagent/machines?*", (route) =>
    route.fulfill({
      json: [
        {
          node_id: "node-1",
          session_id: "s1",
          conversation_id: null,
          status: "agent",
          reason: null,
          display: "secure",
        },
      ],
    }),
  );
  await page.route(
    "**/api/v1/assistant/nyxagent/machines/*/contexts",
    (route) => route.fulfill({ json: [] }),
  );
  await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
  const preview = page.getByRole("complementary", {
    name: "Preview",
    exact: true,
  });
  const rail = page.getByRole("navigation", { name: "Conversation index" });
  async function expectRailInChat() {
    await expect(async () => {
      const transcript = (await page.locator("[data-chat-transcript]").boundingBox())!;
      const toolbar = (await page.locator("[data-chat-header-controls]").boundingBox())!;
      const composer = (await page.locator("[data-composer-dock]").boundingBox())!;
      const bounds = (await rail.boundingBox())!;
      expect(bounds.x + bounds.width).toBeCloseTo(transcript.x + transcript.width, 0);
      expect(bounds.x + bounds.width).toBeLessThanOrEqual((await preview.boundingBox())!.x + 1);
      expect(bounds.y).toBeCloseTo(Math.max(transcript.y, toolbar.y + toolbar.height + 8), 0);
      expect(bounds.y + bounds.height).toBeCloseTo(Math.min(transcript.y + transcript.height, composer.y), 0);
      expect(bounds.height).toBeGreaterThan(300);
    }).toPass();
  }
  const machine = preview.getByRole("region", { name: "Live machine desktop" });
  await expect(machine).toBeVisible();
  const panel = card(page);
  await expect(panel.getByText("Inspecting the browser page")).toBeVisible();
  await expectRailInChat();
  const machineBounds = (await machine.boundingBox())!;
  const cmaBounds = (await panel.boundingBox())!;
  expect(cmaBounds.x).toBeCloseTo(machineBounds.x - 12, 0);
  expect(cmaBounds.width).toBeCloseTo(
    Math.min(500, machineBounds.width + 24),
    0,
  );
  expect(cmaBounds.y).toBeGreaterThan(machineBounds.y + machineBounds.height);
  await expect(
    preview.getByRole("complementary", { name: "CMA sessions", exact: true }),
  ).toHaveCount(1);
  await expect(
    page.getByRole("separator", { name: "Resize preview column" }),
  ).toHaveCount(1);
  const divider = page.getByRole("separator", {
    name: "Resize preview column",
  });
  await divider.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(divider).toHaveAttribute("aria-valuenow", "52");
  await expectRailInChat();
  const resizedPreview = (await preview.boundingBox())!;
  const resizedCmaBounds = (await panel.boundingBox())!;
  expect(resizedCmaBounds.width).toBeCloseTo(
    Math.min(500, resizedPreview.width - 12),
    0,
  );
  expect(resizedCmaBounds.x).toBeCloseTo(resizedPreview.x + 12, 0);
  await expect(composerInput(page)).toBeInViewport({ ratio: 1 });
  await panel.getByRole("button", { name: "Minimise CMA session" }).click();
  await expect(machine).toBeVisible();
  await expect(composerInput(page)).toBeInViewport({ ratio: 1 });
  await page.getByRole("button", { name: "Hide machine", exact: true }).click();
  await expect(machine).toHaveCount(0);
  await expect(preview).toBeVisible();
  await expect(panel.getByRole("status")).toHaveText("Working");
  await expect(divider).toHaveAttribute("aria-valuenow", "52");
  await expectRailInChat();
  const reviewScreenshots = join(homedir(), ".heca/plans/assistant-settings-rail-ui/after");
  await mkdir(reviewScreenshots, { recursive: true });
  await page.screenshot({ path: join(reviewScreenshots, "rail-desktop-cma-only.png"), animations: "disabled" });
  await page.getByRole("button", { name: "Show machine", exact: true }).click();
  await expect(machine).toBeVisible();
  await panel.getByRole("button", { name: "Maximise CMA session" }).click();
  await expectRailInChat();
  await page.screenshot({ path: join(reviewScreenshots, "rail-desktop-cma-and-machine.png"), animations: "disabled" });
  await page.screenshot({ path: "/tmp/nyxid-cma-with-machine.png" });
});

test("keeps unpinned work in its verified source and pins across every route, reload, and source deep link", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const cma = await mockCma(page);
  await openCmaAssistant(page, {
    faults: { nyxagentEnabled: true },
    specialist: true,
  });
  const panel = card(page);
  await expect(panel.getByText("Inspecting the browser page")).toBeVisible();
  await expect(
    page.getByLabel("CMA is working in this session", { exact: true }),
  ).toBeVisible();
  const source = sourceByPage.get(page)!;
  await expect(
    panel.getByRole("link", { name: "Open source chat" }),
  ).toHaveAttribute(
    "href",
    `/assistant?c=${source.conversation}&agent=${source.specialist}`,
  );
  await expect(
    panel.getByRole("link", { name: /View in CMA/ }),
  ).toHaveAttribute("href", "https://cma.example/agents/agt_observed");
  await page.getByRole("button", { name: /^New chat with/ }).click();
  await expect(sessions(page)).toHaveCount(0);
  const stoppedAt = cma.tailCount();
  await page.waitForTimeout(1500);
  expect(cma.tailCount()).toBe(stoppedAt);
  await page.goto(
    `/assistant?mock=1&c=${source.conversation}&agent=${source.specialist}`,
  );
  await expect(panel.getByText("Inspecting the browser page")).toBeVisible();
  await panel.getByRole("button", { name: "Pin CMA session" }).click();
  await page.getByRole("button", { name: /^New chat with/ }).click();
  await expect(
    panel.getByRole("button", { name: "Unpin CMA session (Pinned)" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByLabel("CMA is working in this session", { exact: true }),
  ).toHaveCount(0);
  await page.goto("/keys?mock=1");
  await expect(panel.getByText("Inspecting the browser page")).toBeVisible();
  await page.reload();
  await expect(
    panel.getByRole("button", { name: "Unpin CMA session (Pinned)" }),
  ).toBeVisible();
  await expect(
    panel
      .getByRole("button", { name: "Unpin CMA session (Pinned)" })
      .getByText("Pinned", { exact: true }),
  ).toBeVisible();
  await expect(sessions(page)).toHaveCount(1);
  await panel
    .getByRole("button", { name: "Unpin CMA session (Pinned)" })
    .click();
  await expect(sessions(page)).toHaveCount(0);
  await page.goto(
    `/assistant?mock=1&c=${source.conversation}&agent=${source.specialist}`,
  );
  await expect(panel.getByText("Inspecting the browser page")).toBeVisible();
  // Source content can arrive before the authoritative pin read. Wait for the
  // actual unpinned control before checking its state, not the preceding card.
  await panel.getByRole("button", { name: "Pin CMA session" }).waitFor({ state: "visible" });
  await expect(
    panel.getByRole("button", { name: "Pin CMA session" }),
  ).toHaveAttribute("aria-pressed", "false");
  await expect(panel.getByText("Pinned", { exact: true })).toHaveCount(0);
  // This click exercises the specialist routing, not just its href serialization.
  await panel.getByRole("button", { name: "Pin CMA session" }).click();
  await page.goto("/keys?mock=1");
  await panel.getByRole("link", { name: "Open source chat" }).click();
  await expect(page).toHaveURL(
    new RegExp(`c=${source.conversation}.*agent=${source.specialist}`),
  );
  await expect(composerInput(page)).toBeVisible();
  expect(
    cma.requests
      .filter((path) => /\/agents\?/.test(path))
      .every((path) =>
        /status=(starting|working|waiting_approval|paused)/.test(path),
      ),
  ).toBe(true);
});

test("unattributed CMA inventory and old pin objects never appear as local activity", async ({
  page,
}) => {
  const cma = await mockCma(page);
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page.waitForTimeout(1200);
  await expect(sessions(page)).toHaveCount(0);
  expect(cma.tailCount()).toBe(0);
  expect(cma.requests).toEqual([]);
});

for (const [group, paths] of [
  ["assistant shell routes", [
    "/assistant/plugins", "/assistant/approvals", "/assistant/automations",
    "/assistant/machines", "/assistant/machines/new", "/assistant/machines/pair",
  ]],
  ["CMA shell routes", [
    "/assistant/sandboxes", "/assistant/sandboxes/sandbox-1",
    "/assistant/cma-agents/0dc85e4c-6aaf-47de-9df0-e07a69b8ee19",
  ]],
  ["dashboard and standalone routes", [
    "/keys", "/assistant/machines/node-1/desktop",
    "/assistant/machines/node-1/desktop/", "/assistant/not-a-route",
  ]],
] as const) {
test(`renders one pinned surface on ${group} and restores source-only visibility`, async ({
  page,
}) => {
  await mockCma(page);
  await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
  await card(page).getByRole("button", { name: "Pin CMA session" }).click();
  for (const path of paths) {
    await page.goto(`${path}?mock=1`);
    await expect(sessions(page)).toHaveCount(1);
    await expect(
      card(page).getByRole("button", { name: "Unpin CMA session (Pinned)" }),
    ).toBeVisible();
  }
  await page.reload();
  await expect(sessions(page)).toHaveCount(1);
  await card(page).getByRole("link", { name: "Open source chat" }).click();
  await expect(composerInput(page)).toBeVisible();
  await expect(sessions(page)).toHaveCount(1);
  const sourceUrl = new URL(page.url());
  sourceUrl.searchParams.set("mock", "1");
  await card(page).getByRole("button", { name: "Unpin CMA session (Pinned)" }).click();
  await expect(card(page).getByRole("button", { name: "Pin CMA session", exact: true })).toHaveAttribute("aria-pressed", "false");
  for (const path of paths) {
    await page.goto(`${path}?mock=1`);
    await expect(sessions(page)).toHaveCount(0);
  }
  await page.reload();
  await expect(sessions(page)).toHaveCount(0);
  await page.goto(sourceUrl.href);
  await expect(sessions(page)).toHaveCount(1);
  await expect(card(page).getByRole("button", { name: "Pin CMA session", exact: true })).toHaveAttribute("aria-pressed", "false");
});
}
test("uses only supplied valid callbacks and clears missing, null, query, fragment and malformed links", async ({
  page,
}) => {
  const cma = await mockCma(page, { receiptCallback: null });
  await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
  const link = card(page).getByRole("link", { name: "View in CMA" });
  await expect(
    card(page).getByText("Inspecting the browser page"),
  ).toBeVisible();
  await expect(link).toHaveCount(0);
  cma.setCallback("https://CMA.example:8443/agent/custom-path");
  await page.reload();
  await expect(link).toHaveAttribute(
    "href",
    "https://CMA.example:8443/agent/custom-path",
  );
  for (const value of [
    null,
    "https://cma.example/agent?secret=value",
    "https://cma.example/agent#secret",
    "https://user:pass@cma.example/agent",
    "javascript:alert(1)",
    undefined,
  ]) {
    cma.setCallback(value);
    await page.reload();
    await expect(
      card(page).getByText("Inspecting the browser page"),
    ).toBeVisible();
    await expect(link).toHaveCount(0);
  }
});

for (const cache of ["none", "empty", "error", "transient"] as const) {
  test(`keeps a newly pinned live observation through a slow receipt refresh and client navigation, cached=${cache}`, async ({
    page,
  }) => {
    const cma = await mockCma(page, {
      pinDelayMs: 5000,
      deniedConnection: true,
      pinFailure: cache === "transient" ? 503 : undefined,
    });
    await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
    await expect(
      card(page).getByText("Inspecting the browser page"),
    ).toBeVisible();
    const heads = cma.headCount();
    if (cache !== "none")
      await page.evaluate(async (cached) => {
        const client = (window as unknown as { __nyxQueryClient: QueryClient })
          .__nyxQueryClient;
        const source = client
          .getQueryCache()
          .findAll({ queryKey: ["cma-session"] })
          .find(
            (query) =>
              query.queryKey[2] === "origins" &&
              Array.isArray(query.state.data) &&
              query.state.data.length,
          );
        const key: unknown[] = ["cma-session", source!.queryKey[1], "pins", []];
        // Use the live server-verified identity, not route-derived source metadata.
        const session = (
          source!.state.data as {
            connection: { id: string };
            agentId: string;
          }[]
        )[0]!;
        key[3] = [`${session.connection.id}:${session.agentId}`];
        if (cached === "empty" || cached === "transient")
          client.setQueryData(key, []);
        else
          await client
            .fetchQuery({
              queryKey: key,
              queryFn: async () => {
                throw new Error("Earlier receipt denial");
              },
              retry: false,
            })
            .catch(() => undefined);
      }, cache);
    await page.evaluate(() => {
      const track = window as unknown as {
        cmaPinGaps: number;
        cmaPinSample?: number;
      };
      track.cmaPinGaps = 0;
      track.cmaPinSample = window.setInterval(() => {
        if (!document.querySelector('[aria-label="CMA sessions"]'))
          track.cmaPinGaps++;
      }, 20);
    });
    await card(page).getByRole("button", { name: "Pin CMA session" }).click();
    await page.getByRole("button", { name: /^New chat with/ }).click();
    await expect(
      card(page).getByRole("button", { name: "Unpin CMA session (Pinned)" }),
    ).toBeVisible();
    await page.waitForTimeout(5500);
    await expect(sessions(page)).toHaveCount(1);
    expect(cma.headCount()).toBe(heads);
    const gaps = await page.evaluate(() => {
      const track = window as unknown as {
        cmaPinGaps: number;
        cmaPinSample: number;
      };
      clearInterval(track.cmaPinSample);
      return track.cmaPinGaps;
    });
    expect(gaps).toBe(0);
    await card(page)
      .getByRole("button", { name: "Unpin CMA session (Pinned)" })
      .click();
    await expect(sessions(page)).toHaveCount(0);
  });
}

for (const minimised of [false, true]) {
  for (const input of ["pointer", "keyboard"] as const) {
    test(`keeps Reconnect hittable by ${input} at keyboard height, minimised=${minimised}`, async ({
      page,
    }) => {
      const cma = await mockCma(page, { headStatus: 403, headDelayMs: 200 });
      await page.setViewportSize({ width: 390, height: 300 });
      await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
      const panel = card(page);
      const reconnect = panel.getByRole("button", {
        name: "Reconnect",
        exact: true,
      });
      await expect(reconnect).toBeAttached();
      if (minimised)
        await panel
          .getByRole("button", { name: "Minimise CMA session" })
          .click();
      for (let retry = 0; retry < 3; retry++) {
        await expect(
          panel.getByText("Disconnected", { exact: true }),
        ).toBeVisible();
        await expect(reconnect).toBeEnabled();
        if (input === "keyboard") {
          // Tab from the fixed header; native focus must scroll recovery into view.
          await panel
            .getByRole("button", {
              name: minimised ? "Maximise CMA session" : "Minimise CMA session",
            })
            .focus();
          await page.keyboard.press("Tab");
          await expect(reconnect).toBeFocused();
        } else {
          await reconnect.scrollIntoViewIfNeeded();
        }
        // visible() accepts clipped controls. The actual centre must receive input.
        await expect
          .poll(() =>
            reconnect.evaluate((button) => {
              const rect = button.getBoundingClientRect();
              return button.contains(
                document.elementFromPoint(
                  rect.x + rect.width / 2,
                  rect.y + rect.height / 2,
                ),
              );
            }),
          )
          .toBe(true);
        if (input === "keyboard")
          await page.screenshot({
            path: `/tmp/nyxid-cma-keyboard-recovery-${minimised ? "minimised" : "expanded"}.png`,
          });
        const before = cma.headCount();
        if (input === "keyboard") await page.keyboard.press("Enter");
        else await reconnect.click();
        // The old failed button must leave before asserting the next failure.
        // A head request alone does not prove its 403 response has settled.
        await expect(reconnect).not.toBeAttached();
        await expect.poll(cma.headCount).toBeGreaterThan(before);
        await expect(
          panel.getByText("Disconnected", { exact: true }),
        ).toBeVisible();
        await expect(reconnect).toBeEnabled();
        await expect(composerInput(page)).toBeVisible();
      }
    });
  }
}

test("describes CMA header controls on hover and focus without internal dividers", async ({
  page,
}) => {
  await mockCma(page, { actionCount: 5 });
  await openCmaAssistant(page, { faults: { nyxagentEnabled: true } });
  const panel = card(page);
  await expect(panel.getByRole("listitem")).toHaveCount(3);
  const controls = [
    ["link", "Open source chat", "Open source chat"],
    ["link", "View in CMA", "View in CMA"],
    ["button", "Pin CMA session", "Pin to monitor on every page"],
    ["button", "Minimise CMA session", "Minimise CMA session"],
  ] as const;
  for (const [role, name, copy] of controls) {
    const control = panel.getByRole(role, { name, exact: true });
    await expect(control).not.toHaveAttribute("title");
    await control.hover();
    await expect(
      page.getByRole("tooltip", { name: copy, exact: true }),
    ).toBeVisible();
    await page.mouse.move(0, 0);
    await page.keyboard.press("Escape");
    await expect(
      page.getByRole("tooltip", { includeHidden: true }),
    ).toHaveCount(0);
    await control.focus();
    await expect(
      page.getByRole("tooltip", { name: copy, exact: true }),
    ).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(
      page.getByRole("tooltip", { includeHidden: true }),
    ).toHaveCount(0);
  }
  await panel
    .getByRole("button", { name: "Pin CMA session", exact: true })
    .click();
  await page.mouse.move(0, 0);
  await expect(page.getByRole("tooltip", { includeHidden: true })).toHaveCount(
    0,
  );
  await panel
    .getByRole("button", { name: "Unpin CMA session (Pinned)", exact: true })
    .hover();
  await expect(
    page.getByRole("tooltip", {
      name: "Unpin: show only in the source chat",
      exact: true,
    }),
  ).toBeVisible();
  await page.mouse.move(0, 0);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("tooltip", { includeHidden: true })).toHaveCount(
    0,
  );
  await panel
    .getByRole("button", { name: "Minimise CMA session", exact: true })
    .click();
  await page.mouse.move(0, 0);
  await expect(page.getByRole("tooltip", { includeHidden: true })).toHaveCount(
    0,
  );
  await panel
    .getByRole("button", { name: "Maximise CMA session", exact: true })
    .hover();
  await expect(
    page.getByRole("tooltip", { name: "Maximise CMA session", exact: true }),
  ).toBeVisible();
  await panel
    .getByRole("button", { name: "Maximise CMA session", exact: true })
    .click();
  await expect(panel.locator("hr, [role=separator]")).toHaveCount(0);
  expect(
    await panel.getByRole("listitem").evaluateAll((items) =>
      items.every((item) => {
        const style = getComputedStyle(item);
        return (
          style.borderTopWidth === "0px" && style.borderBottomWidth === "0px"
        );
      }),
    ),
  ).toBe(true);
  expect(
    await panel
      .getByRole("button", { name: "Session details", exact: true })
      .evaluate(
        (button) => getComputedStyle(button.parentElement!).borderTopWidth,
      ),
  ).toBe("0px");
});
