import { expect, test, type Page } from "@playwright/test";
import { openAssistant, sendMessage, composerInput } from "./helpers";
import {
  cmaFrame,
  cmaHead,
  cmaSession,
} from "../src/lib/assistant/cma-session.test-fixtures";

async function mockCma(
  page: Page,
  options: { actionCount?: number; idle?: boolean; multiple?: boolean } = {},
) {
  let active = !options.idle;
  let completed = false;
  let tails = 0;
  const actionCount = options.actionCount ?? 1;
  const requests: string[] = [];
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
    if (url.pathname.endsWith("/events/head"))
      return route.fulfill({
        json: {
          ...cmaHead,
          agent_id: second ? "agt_second" : cmaSession.agentId,
        },
      });
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
  };
}

const sessions = (page: Page) =>
  page.getByRole("complementary", { name: "CMA sessions" });
const card = (page: Page) =>
  sessions(page).getByRole("region", {
    name: `CMA session: ${cmaSession.title}`,
    exact: true,
  });

test("automatically shows running CMA, streams while minimised, restores, and removes finished sessions", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const cma = await mockCma(page);
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  const panel = card(page);
  await expect(panel.getByText("Inspecting the browser page")).toBeVisible();
  await expect(
    sessions(page).getByRole("heading", {
      name: "Your CMA session",
      exact: true,
    }),
  ).toBeVisible();
  await expect(sessions(page).getByText("Across chats")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Pin session", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "CMA session", exact: true }),
  ).toHaveCount(0);
  const main = await page.getByRole("main").boundingBox();
  const bounds = await panel.boundingBox();
  const preview = (await page.getByRole("complementary", { name: "Preview", exact: true }).boundingBox())!;
  expect(bounds!.x).toBeCloseTo(preview.x + 12, 0);
  expect(bounds!.x + bounds!.width).toBeCloseTo(main!.x + main!.width, 0);
  expect((await composerInput(page).boundingBox())!.x + (await composerInput(page).boundingBox())!.width).toBeLessThanOrEqual(preview.x);
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
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await expect
    .poll(
      () =>
        cma.requests.filter((url) => url.includes("/api/v1/agents?")).length,
    )
    .toBeGreaterThan(0);
  await expect(sessions(page)).toHaveCount(0);
  expect(cma.tailCount()).toBe(0);
  cma.start();
  await expect(card(page).getByText("Inspecting the browser page")).toBeVisible(
    { timeout: 12_000 },
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
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    const panel = card(page);
    await expect(
      panel.getByRole("region", { name: "CMA activity" }).getByRole("listitem"),
    ).toHaveCount(30);
    const details = panel.getByRole("button", {
      name: "Session details",
      exact: true,
    });
    const connection = panel.getByText(cmaSession.connection.name, { exact: true });
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
    expect((await composerInput(page).boundingBox())!.y).toBe(composerBefore!.y);
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
    await page.screenshot({ path: `/tmp/nyxid-cma-details-${viewport.width}.png` });
  });
}

test("keeps the automatic CMA card and chat usable together on a narrow screen", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await mockCma(page);
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
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
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  const panel = card(page);
  const actions = panel.getByRole("region", { name: "CMA activity" });
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
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
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
      json: [{
        node_id: "node-1",
        session_id: "s1",
        conversation_id: null,
        status: "agent",
        reason: null,
        display: "secure",
      }],
    }),
  );
  await page.route("**/api/v1/assistant/nyxagent/machines/*/contexts", (route) =>
    route.fulfill({ json: [] }),
  );
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
  const preview = page.getByRole("complementary", { name: "Preview", exact: true });
  const machine = preview.getByRole("region", { name: "Live machine desktop" });
  await expect(machine).toBeVisible();
  const panel = card(page);
  await expect(panel.getByText("Inspecting the browser page")).toBeVisible();
  const machineBounds = (await machine.boundingBox())!;
  const cmaBounds = (await panel.boundingBox())!;
  expect(cmaBounds.x).toBeCloseTo(machineBounds.x - 12, 0);
  expect(cmaBounds.width).toBeCloseTo(machineBounds.width + 24, 0);
  expect(cmaBounds.y).toBeGreaterThan(machineBounds.y + machineBounds.height);
  await expect(preview.getByRole("complementary", { name: "CMA sessions", exact: true })).toHaveCount(1);
  await expect(page.getByRole("separator", { name: "Resize preview column" })).toHaveCount(1);
  const divider = page.getByRole("separator", { name: "Resize preview column" });
  await divider.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(divider).toHaveAttribute("aria-valuenow", "52");
  await expect(composerInput(page)).toBeInViewport({ ratio: 1 });
  await panel.getByRole("button", { name: "Minimise CMA session" }).click();
  await expect(machine).toBeVisible();
  await expect(composerInput(page)).toBeInViewport({ ratio: 1 });
  await page.getByRole("button", { name: "Hide machine", exact: true }).click();
  await expect(machine).toHaveCount(0);
  await expect(preview).toBeVisible();
  await expect(panel.getByRole("status")).toHaveText("Working");
  await expect(divider).toHaveAttribute("aria-valuenow", "52");
  await page.getByRole("button", { name: "Show machine", exact: true }).click();
  await expect(machine).toBeVisible();
  await panel.getByRole("button", { name: "Maximise CMA session" }).click();
  await page.screenshot({ path: "/tmp/nyxid-cma-with-machine.png" });
});
