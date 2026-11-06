import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import type { CmaSandbox } from "../src/schemas/nyxbot-cma-sandbox";

const sandbox: CmaSandbox = {
  id: "sandbox-local-01",
  label: "Research workspace",
  cma_sandbox_id: "sandbox_cma_01939db3-49e7-7d83-9343-75ea0fc4416c",
  generation: 7,
  occupies_account_slot: true,
  admission_state: "associated",
  observation_state: "ready",
  resource_state: "ready",
  machine_included: true,
  cpu_millis: 2000,
  memory_mib: 4096,
  capacity: {
    cpu_millis: 2000,
    memory_mib: 4096,
    source: "profile_revision",
  },
  last_activity_at: "2026-11-03T08:45:00Z",
  last_observed: "2026-11-03T08:46:00Z",
  retire_requested_at: null,
  retired_at: null,
  billing: { provider: "cma", cost_status: "unknown" },
};
const agentId = "agent-local-01";
const remoteAgentId = "agent_cma_01939db3-49e7-7d83-9343-75ea0fc4416c";
const responseId = "response_fixture_exact_01";
const base = "/assistant/nyxagent";
const sandboxBase = `${base}/sandboxes`;
const agentBase = `${base}/cma-agents/${agentId}`;
const exactOutput =
  "Exact task result: the requested release checks are complete.";
const sharedOutput = "Earlier shared Agent history from another NyxBot chat.";
const responseOnlyOutput = "Response-only output must not replace the verified conversation.";

interface FixtureOptions {
  empty?: boolean;
  enabled?: boolean;
  responseBinding?: "exact" | "wrong-agent" | "wrong-response";
  desktopReason?: string;
}

async function cmaApi(page: Page, options: FixtureOptions = {}) {
  const reads: string[] = [];
  const writes: {
    method: string;
    path: string;
    body: unknown;
    key: string | undefined;
  }[] = [];
  const sockets: string[] = [];
  const errors: string[] = [];
  let projection = { ...sandbox };
  page.on("pageerror", (error) => errors.push(error.message));
  // Test mode installs assistant fixtures before page hooks run. Keep its
  // unrelated sidebar data, but send CMA requests through the real HTTP client
  // so Playwright owns the complete transport boundary for these pages.
  await page.addInitScript(() => {
    type Handler = typeof globalThis.__nyxidAssistantHttpMock;
    let handler: Handler;
    Object.defineProperty(globalThis, "__nyxidAssistantHttpMock", {
      configurable: true,
      get: () => (request: Parameters<NonNullable<Handler>>[0]) =>
        /^\/assistant\/nyxagent\/(sandboxes|cma-agents)(\/|\?|$)/.test(
          request.endpoint,
        )
          ? undefined
          : handler?.(request),
      set: (next: Handler) => {
        handler = next;
      },
    });
  });
  // Every WebSocket is handled in process, including Vite's development socket.
  await page.routeWebSocket("**/*", (socket) => {
    const url = new URL(socket.url());
    if (url.pathname === "/") {
      socket.send(JSON.stringify({ type: "connected" }));
    } else {
      sockets.push(socket.url());
    }
    socket.onMessage(() => {});
  });
  // Only static files from this dedicated Vite server may reach the network.
  await page.route("**/*", (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (
      url.origin === "http://localhost:4611" &&
      request.method() === "GET" &&
      ["document", "script", "stylesheet", "image", "font", "other"].includes(
        request.resourceType(),
      ) &&
      !/^\/(api|oauth|mcp|health|\.well-known)(\/|$)/.test(url.pathname)
    )
      return route.continue();
    return route.abort();
  });
  await mockDashboard(page);
  await page.route("**/api/v1/**", (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace("/api/v1", "");
    if (
      request.method() === "GET" &&
      ["/orgs", "/api-keys", "/nodes", "/runtime-config"].includes(path)
    )
      return route.fallback();
    if (!["GET", "HEAD", "OPTIONS"].includes(request.method()))
      writes.push({
        method: request.method(),
        path,
        body: request.postData() ? request.postDataJSON() : null,
        key: request.headers()["idempotency-key"],
      });
    return route.fulfill({
      status: 404,
      json: {
        error: "not_found",
        error_code: 404,
        message: "Unconfigured fixture API blocked locally",
      },
    });
  });
  await page.route("**/api/v1/users/me", (route) =>
    route.fulfill({
      json: {
        id: "test-user",
        email: "fixture@example.test",
        display_name: "Fixture owner",
        is_admin: false,
        is_active: true,
        email_verified: true,
        onboarding_completed: true,
        created_at: "2026-10-01T00:00:00Z",
        capabilities: {
          billing_available: true,
          enabled_features: [
            "experimental:ai-assistant",
            ...(options.enabled === false ? [] : ["assistant:cma-sandboxes"]),
          ],
        },
      },
    }),
  );
  await page.route("**/api/v1/assistant/nyxagent/**", (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.replace("/api/v1", "");
    if (request.method() !== "GET") {
      writes.push({
        method: request.method(),
        path,
        body: request.postData() ? request.postDataJSON() : null,
        key: request.headers()["idempotency-key"],
      });
      if (
        request.method() === "DELETE" &&
        path === `${sandboxBase}/${sandbox.id}`
      ) {
        projection = {
          ...projection,
          admission_state: "retiring",
          observation_state: "retiring",
          resource_state: "retiring",
          retire_requested_at: "2026-11-03T09:00:00Z",
        };
        return route.fulfill({
          status: 202,
          json: { sandbox: projection, pending: true },
        });
      }
      return route.fulfill({
        status: 405,
        json: { message: "Fixture write blocked" },
      });
    }
    reads.push(`${path}${url.search}`);
    if (path === sandboxBase)
      return route.fulfill({
        json: {
          sandboxes: options.empty ? [] : [projection],
          occupied: options.empty ? 0 : 1,
          limit: 1,
          conflict: false,
          inventory_complete: true,
          next_cursor: null,
        },
      });
    if (path === `${sandboxBase}/${sandbox.id}`)
      return route.fulfill({ json: projection });
    if (path === agentBase)
      return route.fulfill({
        json: {
          id: agentId,
          cma_agent_id: remoteAgentId,
          label: "Release research Agent",
          status: "stopped",
          sandbox_id: sandbox.id,
          cma_sandbox_id: sandbox.cma_sandbox_id,
          cma_workspace_id: "workspace_fixture_01",
          cma_worktree_id: null,
          last_observed: sandbox.last_observed,
          available: true,
          unavailable_reason: null,
        },
      });
    if (path === `${agentBase}/responses/${responseId}`)
      return route.fulfill({
        json: {
          id:
            options.responseBinding === "wrong-response"
              ? "other_response"
              : responseId,
          agent_id:
            options.responseBinding === "wrong-agent"
              ? "other_agent"
              : remoteAgentId,
          status: "completed",
          output: [
            {
              id: "output_exact",
              type: "message",
              role: "assistant",
              content: responseOnlyOutput,
            },
          ],
        },
      });
    if (path === `${agentBase}/transcript`)
      return route.fulfill({
        json: {
          messages: [],
          turns: [
            {
              turn_sequence: 10,
              started_at: "2026-11-03T08:00:00Z",
              status: "completed",
              messages: [
                {
                  id: "request_exact",
                  sequence: 9,
                  role: "user",
                  content: "Check the release",
                  created_at: "2026-11-03T08:00:00Z",
                },
                {
                  id: "reply_exact",
                  sequence: 14,
                  role: "assistant",
                  content: exactOutput,
                  created_at: "2026-11-03T08:01:00Z",
                },
              ],
            },
            {
              turn_sequence: 20,
              started_at: "2026-11-03T08:02:00Z",
              status: "completed",
              messages: [
                {
                  id: "request_shared",
                  sequence: 19,
                  role: "user",
                  content: "Inspect the shared workspace",
                  created_at: "2026-11-03T08:02:00Z",
                },
                {
                  id: "shared_message",
                  sequence: 24,
                  role: "assistant",
                  content: sharedOutput,
                  created_at: "2026-11-03T08:03:00Z",
                },
              ],
            },
          ],
          next_cursor: null,
        },
      });
    if (path === `${agentBase}/events/snapshot`)
      return route.fulfill({
        json: {
          events: [
            {
              id: "start_exact",
              sequence: 10,
              type: "turn_started",
              response_id: responseId,
              turn_sequence: 10,
              status: null,
              message: null,
              created_at: "2026-11-03T08:00:00Z",
            },
            {
              id: "start_shared",
              sequence: 20,
              type: "turn_started",
              response_id: "newer_response",
              turn_sequence: 20,
              status: null,
              message: null,
              created_at: "2026-11-03T08:02:00Z",
            },
            {
              id: "event_snapshot",
              sequence: 25,
              cursor: "cursor_snapshot_25",
              type: "tool_completed",
              status: "completed",
              message: "Shared tool completed a prior workspace inspection.",
              created_at: "2026-11-03T08:03:00Z",
            },
          ],
          next_cursor: null,
          through_cursor: "cursor_snapshot_25",
        },
      });
    if (path === `${agentBase}/desktop`)
      return route.fulfill({
        json: {
          available: false,
          reason: options.desktopReason ?? "agent_stopped",
          shared: true,
          width: null,
          height: null,
          websocket_path: null,
        },
      });
    if (
      path === `${agentBase}/events` ||
      path === `${agentBase}/responses/${responseId}/events`
    )
      return route.fulfill({
        contentType: "text/event-stream",
        headers: { "Cache-Control": "no-cache" },
        body: ": local fixture stream closed\n\n",
      });
    return route.fallback();
  });
  return { reads, writes, sockets, errors };
}

async function capture(page: Page, testInfo: TestInfo, name: string) {
  await page.screenshot({
    path: testInfo.outputPath(`${name}.png`),
    fullPage: true,
    animations: "disabled",
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  const dialog = page.getByRole("dialog");
  if (await dialog.count())
    expect(
      await dialog.evaluate(
        (element) => element.scrollWidth <= element.clientWidth + 1,
      ),
    ).toBe(true);
}

for (const width of [1440, 390]) {
  test(`Sandboxes retain the occupied allocation after retirement acceptance at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: width < 768 ? 844 : 1000 });
    const api = await cmaApi(page);
    await page.goto("/assistant/sandboxes");
    const environment = page.getByRole("region", {
      name: `Environment ${sandbox.label}`,
    });
    await expect(environment).toBeVisible();
    await expect(page.getByText("1 / 1 used", { exact: true })).toBeVisible();
    await expect(
      environment.getByText("Running", { exact: true }),
    ).toBeVisible();
    await expect(
      environment.getByText("2 vCPU", { exact: true }),
    ).toBeVisible();
    await expect(environment.getByText("4 GiB", { exact: true })).toBeVisible();
    await expect(
      page.getByText(/Price and accrued charges are unknown here/),
    ).toHaveCount(0);
    await expect(
      page.getByRole("tab", { name: "Desktop", exact: true }),
    ).toHaveCount(0);
    await capture(page, testInfo, "sandboxes-populated");
    await environment
      .getByRole("button", { name: "Retire environment", exact: true })
      .click();
    const dialog = page.getByRole("dialog", {
      name: "Retire environment",
      exact: true,
    });
    const submit = dialog.getByRole("button", {
      name: "Retire environment",
      exact: true,
    });
    await expect(submit).toBeDisabled();
    await dialog
      .getByLabel(`Type ${sandbox.label} to confirm`)
      .fill("Research");
    await expect(submit).toBeDisabled();
    expect(api.writes).toEqual([]);
    await dialog
      .getByLabel(`Type ${sandbox.label} to confirm`)
      .fill(sandbox.label);
    await expect(submit).toBeEnabled();
    await capture(page, testInfo, "retire-confirmation");
    await submit.click();
    await expect(dialog).toHaveCount(0);
    await expect(
      environment.getByText("Retiring", { exact: true }),
    ).toBeVisible();
    await expect(page.getByText("1 / 1 used", { exact: true })).toBeVisible();
    await expect(
      environment.getByText(
        /Your allocation stays in use until retirement is confirmed/,
      ),
    ).toBeVisible();
    await expect(
      environment.getByRole("button", {
        name: "Retire environment",
        exact: true,
      }),
    ).toBeDisabled();
    await expect(
      page.getByText("Room for your next task", { exact: true }),
    ).toHaveCount(0);
    expect(api.writes).toEqual([
      {
        method: "DELETE",
        path: `${sandboxBase}/${sandbox.id}`,
        body: { generation: 7 },
        key: expect.stringMatching(/^[0-9a-f]{8}-[0-9a-f-]{27}$/i),
      },
    ]);
    await capture(page, testInfo, "retirement-pending");
    expect(api.errors).toEqual([]);
    expect(api.sockets).toEqual([]);
  });

  test(`empty Sandboxes are passive at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: width < 768 ? 844 : 1000 });
    const api = await cmaApi(page, { empty: true });
    await page.goto("/assistant/sandboxes");
    await expect(
      page.getByRole("heading", { name: "Room for your next task" }),
    ).toBeVisible();
    await expect(page.getByText("0 / 1 used", { exact: true })).toBeVisible();
    await expect(
      page.getByText(/Simply opening this page doesn’t start one/),
    ).toBeVisible();
    await expect(page.getByRole("link", { name: "Ask NyxBot" })).toBeVisible();
    await expect(page.getByText(/created through NyxID/)).toBeVisible();
    await expect(page.getByText(/NyxID Sandbox allocation/)).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Link existing", exact: true }),
    ).toHaveCount(0);
    await expect(page.getByText(/Already have a CMA/)).toHaveCount(0);
    await capture(page, testInfo, "sandboxes-empty");
    expect(
      api.reads.some((path) => /\/(claim|unregistered)(\?|$)/.test(path)),
    ).toBe(false);
    expect(api.writes).toEqual([]);
    expect(api.errors).toEqual([]);
    expect(api.sockets).toEqual([]);
  });

  test(`Cloud assistant preserves an old link within its unified conversation at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: width < 768 ? 844 : 1000 });
    const api = await cmaApi(page);
    await page.goto(`/assistant/cma-agents/${agentId}?response=${responseId}`);
    await expect(
      page.getByRole("heading", { name: "Release research Agent", level: 1 }),
    ).toBeVisible();
    await expect(
      page.getByRole("tab", { name: "Conversation", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText(/Opened at the request from your chat/),
    ).toBeVisible();
    await expect(page.getByText(exactOutput, { exact: true })).toBeVisible();
    await expect(page.getByText(sharedOutput, { exact: true })).toBeVisible();
    await expect(page.getByText(responseOnlyOutput, { exact: true })).toHaveCount(0);
    await expect(
      page.getByRole("region", { name: "Linked request", exact: true }),
    ).toContainText(exactOutput);
    await expect
      .poll(() =>
        api.reads.includes(
          `${agentBase}/events?after_cursor=cursor_snapshot_25`,
        ),
      )
      .toBe(true);
    expect(api.reads).toContain(`${agentBase}/transcript`);
    expect(api.reads).not.toContain(
      `${agentBase}/responses/${responseId}/events`,
    );
    await capture(page, testInfo, "assistant-linked-request");
    await page.getByRole("button", { name: "Jump to latest", exact: true }).click();
    await expect(page).toHaveURL(
      new RegExp(`/assistant/cma-agents/${agentId}\\?response=${responseId}$`),
    );
    await expect(page.getByText(sharedOutput, { exact: true })).toBeVisible();
    await expect(
      page.getByText(/all NyxBot chats that use this assistant/),
    ).toBeVisible();
    await expect(page.getByText(exactOutput, { exact: true })).toBeVisible();
    await capture(page, testInfo, "assistant-latest-conversation");
    await page.getByRole("tab", { name: "Desktop", exact: true }).click();
    const desktop = page.getByRole("region", {
      name: "Shared Sandbox desktop",
      exact: true,
    });
    await expect(
      desktop.getByText(
        "This Agent is stopped. Its shared desktop connection is unavailable.",
        { exact: true },
      ),
    ).toBeVisible();
    await expect(
      desktop.getByText(/Manual input does not pause the Agent/),
    ).toBeVisible();
    await capture(page, testInfo, "agent-desktop-unavailable");
    expect(api.writes).toEqual([]);
    expect(api.errors).toEqual([]);
    expect(api.sockets).toEqual([]);
  });
}

test("feature-off direct routes make no CMA calls", async ({
  page,
}, testInfo) => {
  const api = await cmaApi(page, { enabled: false });
  await page.goto("/assistant/sandboxes");
  await expect(
    page.getByText("Sandboxes are not enabled for this account.", {
      exact: true,
    }),
  ).toBeVisible();
  await capture(page, testInfo, "sandboxes-flag-off");
  await page.goto(`/assistant/sandboxes/${sandbox.id}`);
  await expect(
    page.getByText("Sandboxes are not enabled for this account.", {
      exact: true,
    }),
  ).toBeVisible();
  await page.goto(
    `/assistant/cma-agents/${agentId}?response=${responseId}&tab=desktop`,
  );
  await expect(
    page.getByText("CMA execution is not enabled for this account.", {
      exact: true,
    }),
  ).toBeVisible();
  expect(
    api.reads.filter(
      (path) => path.startsWith(sandboxBase) || path.includes("/cma-agents/"),
    ),
  ).toEqual([]);
  expect(api.writes).toEqual([]);
  expect(api.sockets).toEqual([]);
  expect(api.errors).toEqual([]);
});

test("a direct assistant entry opens recent conversation without inventing a response binding", async ({
  page,
}, testInfo) => {
  const api = await cmaApi(page);
  await page.goto(`/assistant/cma-agents/${agentId}`);
  await expect(
    page.getByRole("region", { name: "Latest request" }),
  ).toBeVisible();
  await expect(page.getByText(sharedOutput, { exact: true })).toBeVisible();
  await expect(page.getByText(exactOutput, { exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "Linked request", exact: true })).toHaveCount(0);
  expect(api.reads.some((path) => path.includes("/responses/"))).toBe(false);
  await capture(page, testInfo, "assistant-direct-entry");
  expect(api.writes).toEqual([]);
  expect(api.errors).toEqual([]);
});

for (const binding of ["wrong-agent", "wrong-response"] as const) {
  test(`an assistant rejects a ${binding} link while keeping its own conversation`, async ({
    page,
  }, testInfo) => {
    const api = await cmaApi(page, { responseBinding: binding });
    await page.goto(`/assistant/cma-agents/${agentId}?response=${responseId}`);
    await expect(page.getByRole("alert")).toHaveText(
      "The linked request is unavailable or does not belong to this assistant.",
    );
    await expect(page.getByText(responseOnlyOutput, { exact: true })).toHaveCount(0);
    await expect(page.getByText(exactOutput, { exact: true })).toBeVisible();
    await expect(page.getByText(sharedOutput, { exact: true })).toBeVisible();
    await expect(
      page.getByRole("region", { name: "Linked request" }),
    ).toHaveCount(0);
    expect(
      api.reads.some(
        (path) => path.includes("/responses/") && path.endsWith("/events"),
      ),
    ).toBe(false);
    await capture(page, testInfo, `assistant-${binding}`);
    expect(api.writes).toEqual([]);
    expect(api.errors).toEqual([]);
  });
}

test("Desktop without a machine remains passive", async ({
  page,
}, testInfo) => {
  // nyxbot_cma_agent_service::desktop_reason returns this native reason.
  const api = await cmaApi(page, { desktopReason: "desktop_not_enabled" });
  await page.goto(`/assistant/cma-agents/${agentId}?tab=desktop`);
  await expect(
    page.getByText(
      "This environment has no desktop capability. Opening this view does not create one.",
      { exact: true },
    ),
  ).toBeVisible();
  await capture(page, testInfo, "agent-desktop-no-machine");
  expect(api.writes).toEqual([]);
  expect(api.sockets).toEqual([]);
  expect(api.errors).toEqual([]);
});
