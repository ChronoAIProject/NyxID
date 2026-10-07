import { expect, test, type Locator, type Page } from "@playwright/test";

const capacityMessage =
  "Validation error: Agent order storage is full (200 connections across all services). Reset the agent order of another service, or release unavailable preferences for services you can no longer access, then try again.";
const catalog = "aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa";
const slackCatalog = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const group = `catalog:${catalog}`;
const ids = Array.from(
  { length: 202 },
  (_, i) => `${String(i + 1).padStart(8, "0")}-1111-4111-8111-111111111111`,
);

async function assertOrderActionsNearDiscovery(scope: Locator) {
  const { summaryBox, actionsBox } = await scope.evaluate((element) => {
    const summary = element.querySelector("[data-service-order-summary]");
    const actions = element.querySelector(
      '[role="group"][aria-label="Discovery order actions for Anthropic"]',
    );
    if (!summary || !actions) throw new Error("Discovery summary/actions missing");
    const bounds = (target: Element) => {
      const { x, y, width, height } = target.getBoundingClientRect();
      return { x, y, width, height };
    };
    return { summaryBox: bounds(summary), actionsBox: bounds(actions) };
  });
  if (actionsBox.y < summaryBox.y + summaryBox.height) {
    const gap = actionsBox.x - summaryBox.x - summaryBox.width;
    expect(gap).toBeGreaterThanOrEqual(8);
    expect(gap).toBeLessThanOrEqual(16);
  } else {
    expect(Math.abs(actionsBox.x - summaryBox.x)).toBeLessThanOrEqual(1);
    expect(
      actionsBox.y - summaryBox.y - summaryBox.height,
    ).toBeLessThanOrEqual(8);
  }
}

function service(i: number, slack = false) {
  return {
    id: slack ? "99999999-9999-4999-8999-999999999999" : ids[i],
    label: slack
      ? "Slack workspace"
      : i < 2
        ? "Anthropic"
        : i === 2
          ? "Anthropic production organization connection with a long label"
          : `Anthropic ${i + 1}`,
    slug: slack ? "slack" : `llm-anthropic-${i + 1}`,
    catalog_service_id: slack ? slackCatalog : catalog,
    catalog_service_slug: slack ? "api-slack" : "llm-anthropic",
    catalog_service_name: slack ? "Slack" : "Anthropic",
    endpoint_url: "https://example.com",
    endpoint_id: `ep-${i}`,
    credential_type: "bearer",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: i === 3 ? "revoked" : "active",
    is_active: slack || i < 4,
    service_type: "http",
    node_id: null,
    node_priority: 0,
    ws_frame_injections: [],
    auto_connected: false,
    expires_at: null,
    last_used_at: null,
    error_message: null,
    created_at: "2026-10-07T00:00:00Z",
    inference: slack
      ? null
      : {
          wire_protocol: "anthropic",
          models: [],
          binding: "user",
          status_slug: "anthropic",
        },
    credential_source:
      i === 2 && !slack
        ? {
            type: "org",
            org_id: "44444444-4444-4444-8444-444444444444",
            org_name: "A very long organization name for mobile wrapping",
            role: "admin",
            allowed: true,
          }
        : { type: "personal" },
  };
}
async function fixture(page: Page, count = 30) {
  const state = {
    count,
    ordered: [ids[0]!, ids[4]!, ids[2]!, ids[1]!],
    version: 1,
    failRead: false,
    gone: false,
    failInventory: false,
    failSave: false,
    conflict: false,
    capacity: false,
    stale: false,
    removed: [] as string[],
    disabled: [] as number[],
    delayInventory: 0,
    delaySave: 0,
    legacy: false,
    includeSlack: true,
    writes: [] as {
      group: string;
      ordered: string[];
      expected_version: number;
    }[],
    releases: [] as unknown[],
    viewWrites: [] as unknown[],
  };
  await page.route("**/api/v1/**", async (route) => {
    const req = route.request();
    const path = new URL(req.url()).pathname;
    let status = 200;
    let body: unknown = {};
    const rows = Array.from({ length: state.count }, (_, i) => {
      const row = service(i);
      if (state.disabled.includes(i)) row.is_active = false;
      const stored = state.ordered.filter(
        (id) => ids.indexOf(id) < state.count && !state.removed.includes(id),
      );
      const active = stored.filter(
        (id) =>
          ids.indexOf(id) < 4 && !state.disabled.includes(ids.indexOf(id)),
      );
      const position = stored.indexOf(row.id!);
      const rank = row.is_active ? active.indexOf(row.id!) : -1;
      if (state.legacy && i === 2)
        Reflect.deleteProperty(row, "credential_source");
      return {
        ...row,
        preference_rank: rank < 0 ? null : rank + 1,
        preference_position: position < 0 ? null : position + 1,
      };
    }).filter((row) => !state.removed.includes(row.id!));
    const response = () => ({
      groups: state.ordered.some(
        (id) => ids.indexOf(id) < state.count && !state.removed.includes(id),
      )
        ? [
            {
              group,
              ordered: state.ordered.filter(
                (id) =>
                  ids.indexOf(id) < state.count && !state.removed.includes(id),
              ),
            },
          ]
        : [],
      version: state.version,
      updated_at: null,
    });
    if (path === "/api/v1/users/me")
      body = {
        id: "human",
        display_name: "Human",
        email: "human@example.com",
        is_active: true,
        role: "user",
        email_verified: true,
        feature_flags: {},
        profile_config: {
          onboarding: { ai_services_completed_at: "2026-10-07T00:00:00Z" },
          services_view: {
            search: "",
            organization_ids: [],
            service_group_ids: [],
            source: "all",
            state: "all",
            service_type: "all",
            show_auto_connected: false,
          },
        },
      };
    else if (path === "/api/v1/users/me/preferences/services") {
      state.viewWrites.push(req.postDataJSON());
      body = req.postDataJSON();
    } else if (path === "/api/v1/service-preferences") {
      status = state.failRead ? 503 : state.gone ? 404 : 200;
      body = response();
    } else if (path.startsWith("/api/v1/service-preferences/groups/")) {
      const write = req.postDataJSON() as {
        ordered: string[];
        expected_version: number;
      };
      if (state.delaySave)
        await new Promise((resolve) => setTimeout(resolve, state.delaySave));
      state.writes.push({
        group: decodeURIComponent(path.split("/").at(-1)!),
        ...write,
      });
      if (state.conflict) {
        state.conflict = false;
        state.version++;
        status = 409;
      } else if (state.capacity) status = 400;
      else if (state.stale) status = 400;
      else if (state.failSave) status = 503;
      else if (write.expected_version !== state.version) status = 409;
      else {
        state.ordered = write.ordered;
        state.version++;
      }
      body = response();
    } else if (path === "/api/v1/service-preferences/hidden") {
      state.releases.push(req.postDataJSON());
      state.capacity = false;
      state.version++;
      body = response();
    } else if (path === "/api/v1/keys") {
      if (state.delayInventory)
        await new Promise((resolve) =>
          setTimeout(resolve, state.delayInventory),
        );
      status = state.failInventory ? 503 : 200;
      body = {
        keys: state.includeSlack
          ? [...rows.slice(0, 1), service(0, true), ...rows.slice(1)]
          : rows,
      };
    } else if (path === "/api/v1/user-services")
      body = {
        services: state.legacy
          ? [{ id: ids[2], credential_source: service(2).credential_source }]
          : [],
      };
    else if (path === "/api/v1/service-insights")
      body = {
        connections: [
          {
            service_id: ids[0],
            billing: null,
            usage: {
              access: {
                visibility: "own_keys",
                basis: "resolved",
                truncated: false,
                keys: Array.from({ length: 5 }, (_, i) => ({
                  id: `agent-${i}`,
                  name: `Agent ${i}`,
                  platform: null,
                  owner_id: "human",
                  permission: "explicit",
                  credential_override: false,
                })),
              },
              activity: {
                visibility: "own_keys",
                period_days: 30,
                tracking: "enabled",
                request_count: 0,
                requests: [],
                truncated: false,
              },
            },
          },
        ],
      };
    else if (path.endsWith("/service-pools"))
      body = {
        pools: new URL(req.url()).searchParams.has("org_id")
          ? []
          : [
              {
                id: "pool",
                user_id: "human",
                name: "Explicit route",
                slug: "explicit-route",
                strategy: "priority",
                members: [
                  {
                    user_service_id: ids[2],
                    enabled: true,
                    weight: 1,
                    priority: 7,
                  },
                ],
                rr_counter: 0,
                is_active: true,
                created_at: "2026-10-07",
                updated_at: "2026-10-07",
              },
            ],
      };
    else if (path === "/api/v1/nodes") body = { nodes: [] };
    else if (path === "/api/v1/orgs") body = { orgs: [] };
    else if (path === "/api/v1/catalog") body = { entries: [] };
    else if (/^\/api\/v1\/keys\/[^/]+\/history$/.test(path))
      body = {
        service_id: path.split("/")[4],
        next_cursor: null,
        tracked_since: null,
        legacy: false,
        deleted: false,
        groups: [],
      };
    else if (path === "/api/v1/public/config")
      body = { telemetry_dsn: null, telemetry_share_analytics: false };
    else {
      status = 404;
    }
    if (status !== 200)
      body = {
        error: "fixture",
        error_code: status === 409 ? 1004 : 1000,
        message: state.capacity
          ? capacityMessage
          : state.stale
            ? "unknown service id"
            : "Fixture read or save failure",
      };
    await route.fulfill({ status, json: body });
  });
  await page.goto("/keys");
  await expect(
    page.getByRole("heading", { name: "Services & Credentials" }),
  ).toBeVisible();
  return state;
}
function card(page: Page) {
  return page.getByRole("region", { name: "Anthropic", exact: true });
}
function row(page: Page, i: number) {
  return page.locator(`[data-service-connection-row="${ids[i]}"]`);
}
async function expand(page: Page) {
  await card(page)
    .getByRole("button", { name: "Expand Anthropic connections", exact: true })
    .click();
  await expect(card(page).getByRole("table")).toBeVisible();
}
async function enter(page: Page) {
  await expand(page);
  await card(page)
    .getByRole("button", { name: "Reorder discovery", exact: true })
    .click();
  await expect(
    card(page).getByRole("form", { name: "Agent order for Anthropic" }),
  ).toBeAttached();
  await expect(
    row(page, 0).getByRole("button", { name: /^Drag / }),
  ).toBeEnabled();
}
async function move(page: Page, i: number) {
  await row(page, i)
    .getByRole("button", { name: /^Move .* up$/ })
    .click();
}
async function drag(page: Page, from: number, to: number) {
  const handle = row(page, from).getByRole("button", { name: /^Drag / });
  await handle.hover();
  const a = await handle.boundingBox();
  const b = await row(page, to)
    .getByRole("button", { name: /^Drag / })
    .boundingBox();
  if (!a || !b) throw Error("No handles");
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  await page.mouse.move(a.x + a.width / 2 + 8, a.y + a.height / 2, {
    steps: 2,
  });
  await expect(handle).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.locator('[role="status"][aria-live="assertive"]'),
  ).toHaveCount(1);
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 15 });
  await expect(
    page.getByRole("status").filter({ hasText: /^Moved to position/ }),
  ).toBeVisible();
  await page.mouse.up();
}
async function focusRefetch(page: Page) {
  await page.evaluate(() =>
    window.dispatchEvent(new Event("visibilitychange")),
  );
}

async function assertReadableIdentity(page: Page, i: number) {
  const cell = row(page, i).locator("td").first();
  const label = cell.locator(`span[title="${service(i).label}"]`);
  const readiness = cell.getByText("Credential check needed", { exact: true });
  const pill = cell.getByLabel(/Discovery preference/);
  const nameBox = await label.boundingBox();
  const readinessBox = await readiness.boundingBox();
  const pillBox = await pill.boundingBox();
  expect(nameBox).not.toBeNull();
  expect(readinessBox).not.toBeNull();
  expect(pillBox).not.toBeNull();
  expect(nameBox!.width).toBeGreaterThanOrEqual(64);
  const overlaps = (
    a: NonNullable<typeof nameBox>,
    b: NonNullable<typeof nameBox>,
  ) =>
    a.x < b.x + b.width &&
    a.x + a.width > b.x &&
    a.y < b.y + b.height &&
    a.y + a.height > b.y;
  expect(overlaps(nameBox!, readinessBox!)).toBe(false);
  expect(overlaps(nameBox!, pillBox!)).toBe(false);
  expect(overlaps(readinessBox!, pillBox!)).toBe(false);
  expect(pillBox!.y).toBeGreaterThanOrEqual(
    Math.max(
      nameBox!.y + nameBox!.height,
      readinessBox!.y + readinessBox!.height,
    ),
  );
  await expect(cell.locator(`code[title="${service(i).slug}"]`)).toHaveText(
    service(i).slug,
  );
}

test("mouse inline 30/26 order, disabled position, persistence and delayed focus", async ({
  page,
}, testInfo) => {
  const state = await fixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await enter(page);
  await expect(
    page.getByRole("button", { name: "Reorder", exact: true }),
  ).toHaveCount(0);
  await expect(
    card(page).getByText(/30 connections while ordering/),
  ).toBeVisible();
  await expect(row(page, 4).getByLabel(/Saved order position 2/)).toHaveText(
    "Saved #2 · disabled",
  );
  await expect(
    card(page).getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await drag(page, 4, 0);
  await expect(row(page, 4).getByLabel(/Saved order position 1/)).toBeVisible();
  await expect(
    row(page, 0).getByLabel(/Discovery preference 1 for/),
  ).toHaveText("Discovery #1");
  const label = row(page, 0).locator('span[title="Anthropic"]');
  const labelBox = await label.boundingBox();
  expect(labelBox?.width).toBeGreaterThanOrEqual(64);
  const pillBox = await row(page, 0)
    .getByLabel(/Discovery preference 1 for/)
    .boundingBox();
  expect(pillBox!.y).toBeGreaterThanOrEqual(labelBox!.y + labelBox!.height);
  await page.screenshot({
    path: testInfo.outputPath("inline-desktop.png"),
    fullPage: true,
  });
  await testInfo.attach("inline desktop", {
    path: testInfo.outputPath("inline-desktop.png"),
    contentType: "image/png",
  });
  state.delayInventory = 250;
  await card(page).getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    card(page).getByRole("button", { name: "Reorder discovery", exact: true }),
  ).toBeFocused();
  expect(state.writes[0]?.group).toBe(group);
  expect(state.writes[0]?.ordered).toHaveLength(30);
  expect(state.writes[0]?.ordered[0]).toBe(ids[4]);
  expect(state.viewWrites).toEqual([]);
  await page.reload();
  await expect(
    card(page).getByRole("button", {
      name: "Preferred: Anthropic",
      exact: true,
    }),
  ).toBeVisible();
  await expand(page);
  await card(page)
    .getByRole("button", { name: "Reorder discovery", exact: true })
    .click();
  await card(page).getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(
    card(page).getByRole("button", { name: "Reorder discovery", exact: true }),
  ).toBeFocused();
});
test("keyboard drag announces once and Escape cancels, move-disabled and confirmed reset", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  const handle = row(page, 0).getByRole("button", { name: /^Drag / });
  await handle.focus();
  await page.keyboard.press("Space");
  await expect(page.locator('[id^="DndLiveRegion-"]')).toContainText(
    "Picked up",
  );
  await page.keyboard.press("ArrowDown");
  await expect(page.locator('[id^="DndLiveRegion-"]')).toContainText(
    "Moved to position 2",
  );
  await page.keyboard.press("Escape");
  await expect(
    row(page, 0).getByLabel(/Discovery preference 1 for/),
  ).toBeVisible();
  await expect(
    card(page).getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await handle.focus();
  await page.keyboard.press("Space");
  await expect(page.locator('[id^="DndLiveRegion-"]')).toContainText(
    "Picked up",
  );
  await page.keyboard.press("ArrowDown");
  await expect(page.locator('[id^="DndLiveRegion-"]')).toContainText(
    "Moved to position 2",
  );
  await page.keyboard.press("Space");
  await expect(page.locator('[id^="DndLiveRegion-"]')).toContainText(
    "Dropped at position 2",
  );
  await expect(
    card(page).getByRole("button", { name: "Save", exact: true }),
  ).toBeEnabled();
  expect(await page.locator('[id^="DndLiveRegion-"]').count()).toBe(1);
  await card(page)
    .getByRole("button", { name: "Move disabled to end" })
    .click();
  await expect
    .poll(() =>
      page
        .locator("[data-ordering-row]")
        .evaluateAll((rows) =>
          rows.slice(0, 4).map((row) => row.getAttribute("data-ordering-row")),
        ),
    )
    .toEqual(expect.arrayContaining(ids.slice(0, 4)));
  page.once("dialog", (dialog) => dialog.dismiss());
  await card(page)
    .getByRole("button", { name: "Reset to default", exact: true })
    .click();
  expect(state.writes).toHaveLength(0);
  page.once("dialog", (dialog) => dialog.accept());
  await card(page)
    .getByRole("button", { name: "Reset to default", exact: true })
    .click();
  await card(page).getByRole("button", { name: "Save", exact: true }).click();
  expect(state.writes[0]?.ordered).toEqual([]);
});
test("touch drag, readable editing identities and contained tablet/mobile table overflow", async ({
  browser,
  baseURL,
}, testInfo) => {
  const context = await browser.newContext({
    baseURL,
    viewport: { width: 390, height: 844 },
    hasTouch: true,
    isMobile: true,
  });
  const page = await context.newPage();
  await fixture(page);
  await enter(page);
  const handle = row(page, 4).getByRole("button", { name: /^Drag / });
  await handle.scrollIntoViewIfNeeded();
  const a = await handle.boundingBox();
  const b = await row(page, 0)
    .getByRole("button", { name: /^Drag / })
    .boundingBox();
  expect(a).not.toBeNull();
  expect(b).not.toBeNull();
  const session = await context.newCDPSession(page);
  await session.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ x: a!.x + a!.width / 2, y: a!.y + a!.height / 2 }],
  });
  for (let i = 1; i <= 12; i++)
    await session.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [
        {
          x: a!.x + a!.width / 2,
          y: a!.y + a!.height / 2 + ((b!.y - a!.y) * i) / 12,
        },
      ],
    });
  await session.send("Input.dispatchTouchEvent", {
    type: "touchEnd",
    touchPoints: [],
  });
  await expect(row(page, 4).getByLabel(/Saved order position 1/)).toBeVisible();
  for (const width of [390, 1024]) {
    await page.setViewportSize({ width, height: 844 });
    const dimensions = await card(page)
      .getByRole("table")
      .evaluate((table) => {
        const parent = table.parentElement!;
        return {
          overflow: parent.scrollWidth > parent.clientWidth,
          root: document.documentElement.scrollWidth <= window.innerWidth,
        };
      });
    expect(dimensions).toEqual({ overflow: true, root: true });
    const labelBox = await row(page, 0)
      .locator('span[title="Anthropic"]')
      .boundingBox();
    expect(labelBox?.width).toBeGreaterThanOrEqual(64);
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: testInfo.outputPath("inline-mobile.png"),
    fullPage: true,
  });
  await testInfo.attach("inline mobile", {
    path: testInfo.outputPath("inline-mobile.png"),
    contentType: "image/png",
  });
  await context.close();
});
test("filtered editing keeps complete group, org/pool metadata and saved view untouched", async ({
  page,
}) => {
  const state = await fixture(page);
  await expand(page);
  const search = page.getByRole("textbox", {
    name: "Search services and connections",
  });
  await search.fill("llm-anthropic-1");
  await search.press("Enter");
  await expect(row(page, 2)).toHaveCount(0);
  await card(page).getByRole("button", { name: "Reorder discovery", exact: true }).click();
  await expect(card(page).getByRole("form")).toBeAttached();
  await move(page, 2);
  await search.fill("Slack");
  await search.press("Enter");
  await expect(
    card(page).getByText("Kept visible while ordering"),
  ).toBeVisible();
  await expect(card(page).getByRole("table")).toBeVisible();
  await expect(
    row(page, 2).getByText("Explicit route · Priority 7"),
  ).toBeVisible();
  await expect(
    row(page, 2).getByText(
      "A very long organization name for mobile wrapping",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Slack", exact: true }),
  ).toBeVisible();
  expect(state.viewWrites).toEqual([]);
  await expect(
    card(page).getByRole("button", { name: "Save", exact: true }),
  ).toBeEnabled();
  await card(page).getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(card(page)).toHaveCount(0);
});
test("guards collapse, another group, view and tab before mutation with one confirm", async ({
  page,
}) => {
  await fixture(page);
  await enter(page);
  await move(page, 2);
  let prompts = 0;
  page.on("dialog", async (dialog) => {
    prompts++;
    await dialog.dismiss();
  });
  await page.locator("main").evaluate((main) => main.scrollTo({ top: 0 }));
  await card(page)
    .getByRole("button", { name: "Collapse Anthropic connections" })
    .click();
  await expect(card(page).getByRole("form")).toBeAttached();
  await page
    .getByRole("region", { name: "Slack", exact: true })
    .getByRole("button", { name: "Expand Slack connections" })
    .click();
  await expect(card(page).getByRole("form")).toBeAttached();
  await page.getByRole("button", { name: "Table view", exact: true }).click();
  await expect(card(page).getByRole("form")).toBeAttached();
  const before = prompts;
  await page.getByRole("tab", { name: "Service Pools" }).click();
  expect(prompts - before).toBe(1);
  expect(new URL(page.url()).searchParams.get("tab")).not.toBe("pools");
  const navigation = prompts;
  await page.getByRole("link", { name: "Settings", exact: true }).click();
  expect(prompts - navigation).toBe(1);
  expect(new URL(page.url()).pathname).toBe("/keys");
  await expect(card(page).getByRole("form")).toBeAttached();
});
test("409 Overwrite persists and Reload resets only current group", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await move(page, 2);
  state.conflict = true;
  await card(page).getByRole("button", { name: "Save", exact: true }).click();
  await card(page)
    .getByRole("button", { name: "Overwrite", exact: true })
    .click();
  await expect(card(page).getByRole("form")).toHaveCount(0);
  expect(state.writes).toHaveLength(2);
  expect(state.writes[1]?.expected_version).toBe(2);
  await page.locator("main").evaluate((main) => main.scrollTo({ top: 0 }));
  await card(page)
    .getByRole("button", { name: "Reorder discovery", exact: true })
    .click();
  await move(page, 1);
  state.conflict = true;
  await card(page).getByRole("button", { name: "Save", exact: true }).click();
  await card(page)
    .getByRole("button", { name: "Reload order", exact: true })
    .click();
  await expect(
    card(page).getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
});
test("capacity release confirmation cancellation and retry-save keep draft", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await move(page, 2);
  state.capacity = true;
  await card(page).getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    card(page).getByRole("button", { name: "Release unavailable preferences" }),
  ).toBeVisible();
  await expect(
    card(page).getByText(capacityMessage, { exact: true }),
  ).toHaveCount(1);
  await expect(
    card(page).getByText(capacityMessage, { exact: true }),
  ).toBeVisible();
  page.once("dialog", (dialog) => dialog.dismiss());
  await card(page)
    .getByRole("button", { name: "Release unavailable preferences" })
    .click();
  expect(state.releases).toHaveLength(0);
  page.once("dialog", async (dialog) => {
    expect(dialog.message()).toContain("if access returns");
    await dialog.accept();
  });
  await card(page)
    .getByRole("button", { name: "Release unavailable preferences" })
    .click();
  await card(page)
    .getByRole("button", { name: "Retry save", exact: true })
    .click();
  await expect(card(page).getByRole("form")).toHaveCount(0);
  expect(state.releases).toEqual([{ expected_version: 1 }]);
  expect(state.writes[1]?.expected_version).toBe(2);
});
test("network recovery keeps rows and successful inventory refresh appends/prunes", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await move(page, 2);
  state.failSave = true;
  await card(page).getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    card(page).getByText("Could not save agent order. Your edits are kept."),
  ).toBeVisible();
  state.failSave = false;
  state.removed = [ids[1]!];
  state.count = 31;
  await focusRefetch(page);
  await expect(row(page, 1)).toHaveCount(0);
  await expect(row(page, 30).getByText("New", { exact: true })).toBeVisible();
  await card(page).getByRole("button", { name: "Save", exact: true }).click();
  await expect(card(page).getByRole("form")).toHaveCount(0);
  expect(state.writes[1]?.ordered).not.toContain(ids[1]);
  expect(state.writes[1]?.ordered).toContain(ids[30]);
});
test("actual stale-ID 400 only prunes after successful inventory Retry", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await move(page, 2);
  state.stale = true;
  await card(page).getByRole("button", { name: "Save", exact: true }).click();
  state.failInventory = true;
  await card(page)
    .getByText(
      "Some connections are no longer available. Refresh connections before saving again.",
    )
    .locator("..")
    .getByRole("button", { name: "Retry", exact: true })
    .click();
  await expect(card(page).getByRole("form")).toBeAttached();
  await expect(row(page, 1)).toBeVisible();
  await expect(
    card(page).getByText(
      "Could not refresh connections. Your edits are kept. Retry inventory refresh.",
    ),
  ).toBeVisible({ timeout: 15000 });
  state.failInventory = false;
  state.stale = false;
  state.removed = [ids[1]!];
  await card(page)
    .getByText(
      "Some connections are no longer available. Refresh connections before saving again.",
    )
    .locator("..")
    .getByRole("button", { name: "Retry", exact: true })
    .click();
  await expect(row(page, 1)).toHaveCount(0);
  await card(page).getByRole("button", { name: "Save", exact: true }).click();
  await expect(card(page).getByRole("form")).toHaveCount(0);
});
test("failed inventory refetch retains card and has visible Retry recovery", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await move(page, 2);
  state.failInventory = true;
  await focusRefetch(page);
  await expect(
    page.getByText("Failed to refresh services. Your edits are kept."),
  ).toBeVisible({ timeout: 15000 });
  await expect(card(page).getByRole("form")).toBeAttached();
  await expect(
    card(page).getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  state.failInventory = false;
  await card(page)
    .getByRole("button", { name: "Retry reads", exact: true })
    .click();
  await expect(
    card(page).getByRole("button", { name: "Save", exact: true }),
  ).toBeEnabled();
});
test("production404/loading/read errors keep explanation and known pills honest", async ({
  page,
}) => {
  const state = await fixture(page);
  state.gone = true;
  await page.reload();
  await expand(page);
  await expect(
    card(page).getByText("Preferred in discovery:", { exact: true }),
  ).toBeVisible();
  await expect(
    row(page, 0).getByLabel(/Discovery preference 1 for/),
  ).toBeVisible();
  await expect(
    card(page).getByRole("button", { name: "Reorder discovery", exact: true }),
  ).toBeDisabled();
  expect(state.writes).toEqual([]);
  state.ordered = [];
  state.gone = false;
  state.failRead = true;
  await page.reload();
  await expand(page);
  await expect(
    card(page).getByText(/^Agent order could not be loaded/),
  ).toBeVisible();
  await expect(
    card(page).getByText("Default server discovery order", { exact: true }),
  ).toHaveCount(0);
  state.failRead = false;
  await card(page).getByRole("button", { name: "Retry reads" }).click();
  await expect(
    card(page).getByText("Default server discovery order", { exact: true }),
  ).toBeVisible();
});

for (const surface of ["card", "overview"] as const) {
  test(`${surface} info tooltip preserves production404 and dirty-draft state on hover and keyboard`, async ({
    page,
  }) => {
    const state = await fixture(page);
    state.gone = true;
    await page.setViewportSize({ width: 390, height: 844 });
    if (surface === "overview")
      await page.goto(`/keys/services/${encodeURIComponent(group)}`);
    else {
      await page.reload();
      await expand(page);
    }
    const section = page.getByRole("region", {
      name: "Agent discovery order for Anthropic",
      exact: true,
    });
    const help = section.getByRole("button", { name: "How discovery order works", exact: true });
    const tooltip = page.locator("[data-service-order-help]");
    const url = page.url();
    await expect(tooltip).toHaveCount(0);
    await expect(page.getByText("Connections that match equally are shown in your order.")).toHaveCount(0);
    await expect(section.getByText("Saving agent order requires the backend update.", { exact: true })).toBeVisible();
    const action = section.getByRole("button", { name: "Reorder discovery", exact: true });
    await expect(action).toHaveCount(1);
    await expect(action).toBeDisabled();
    await expect(action).toHaveAttribute("title", "Saving agent order requires the backend update");
    const icon = help.locator("svg");
    await expect(icon).toHaveCount(1);
    await expect(icon).toHaveClass(/lucide-info/);
    await expect(icon).toHaveCSS("width", "14px");
    await expect(icon).toHaveCSS("height", "14px");
    for (const width of [390, 1024, 1440]) {
      await page.setViewportSize({ width, height: 844 });
      await assertOrderActionsNearDiscovery(section);
      for (const i of [0, 1, 2]) await assertReadableIdentity(page, i);
    }
    await page.setViewportSize({ width: 390, height: 844 });
    await help.hover();
    await expect(tooltip).toBeVisible();
    await help.click();
    await expect(tooltip).toBeVisible();
    await expect(page.getByRole("tooltip").getByRole("listitem")).toHaveText([
      "Connections that match equally are shown in your order.",
      "The AI chooses which connection to use; this order is a preference.",
      "Agents see only the enabled HTTP connections they can access.",
    ]);
    await expect(page.getByRole("tooltip")).toContainText("Provider gateway routing is separate from this discovery order.");
    expect((await page.getByRole("tooltip").innerText()).trim().split(/\s+/).length).toBeLessThanOrEqual(85);
    await expect(tooltip.getByRole("link")).toHaveCount(0);
    await expect(tooltip.getByRole("button")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(tooltip).toHaveCount(0);
    await page.mouse.move(0, 0);
    await help.focus();
    await expect(tooltip).toBeVisible();
    await page.keyboard.press("Enter");
    await expect(tooltip).toBeVisible();
    await expect(help).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(tooltip).toHaveCount(0);
    await expect(help).toBeFocused();
    await help.blur();
    await help.focus();
    await expect(tooltip).toBeVisible();
    await page.keyboard.press("Tab");
    await expect(tooltip).toHaveCount(0);
    expect(page.url()).toBe(url);
    expect(state.writes).toEqual([]);
    expect(state.viewWrites).toEqual([]);
    state.gone = false;
    await page.reload();
    if (surface === "card") await expand(page);
    await action.click();
    await move(page, 2);
    for (const width of [390, 1024, 1440]) {
      await page.setViewportSize({ width, height: 844 });
      for (const i of [0, 1, 2]) await assertReadableIdentity(page, i);
    }
    await page.setViewportSize({ width: 390, height: 844 });
    const draft = await page
      .locator("[data-ordering-row]")
      .evaluateAll((rows) =>
        rows.map((row) => row.getAttribute("data-ordering-row")),
      );
    const beforeSelections = await page
      .getByRole("button")
      .evaluateAll((buttons) =>
        buttons
          .map((button) => button.getAttribute("aria-label"))
          .filter(
            (label) =>
              label?.startsWith("Service view:") ||
              label?.startsWith("Auto-connected services:"),
          ),
      );
    const beforeSearch =
      surface === "card"
        ? await page
            .getByRole("textbox", { name: "Search services and connections" })
            .inputValue()
        : undefined;
    await help.focus();
    await expect(page.getByRole("tooltip").getByRole("listitem")).toHaveCount(3);
    await page.keyboard.press("Escape");
    await expect(tooltip).toHaveCount(0);
    await expect(help).toBeFocused();
    await help.click();
    await section.getByText("4 enabled · 26 disabled", { exact: true }).click();
    await expect(tooltip).toHaveCount(0);
    expect(
      await page
        .locator("[data-ordering-row]")
        .evaluateAll((rows) =>
          rows.map((row) => row.getAttribute("data-ordering-row")),
        ),
    ).toEqual(draft);
    await expect(
      (surface === "card" ? card(page) : page).getByRole("button", {
        name: "Save",
        exact: true,
      }),
    ).toBeEnabled();
    expect(page.url()).toBe(url);
    if (surface === "card")
      expect(
        await page
          .getByRole("textbox", { name: "Search services and connections" })
          .inputValue(),
      ).toBe(beforeSearch);
    expect(
      await page
        .getByRole("button")
        .evaluateAll((buttons) =>
          buttons
            .map((button) => button.getAttribute("aria-label"))
            .filter(
              (label) =>
                label?.startsWith("Service view:") ||
                label?.startsWith("Auto-connected services:"),
            ),
        ),
    ).toEqual(beforeSelections);
    expect(state.writes).toEqual([]);
    expect(state.viewWrites).toEqual([]);
  });
}
test("touch info tooltip toggles and dismisses without changing a dirty 30-row order", async ({
  browser,
}, testInfo) => {
  const context = await browser.newContext({
    hasTouch: true,
    isMobile: true,
    viewport: { width: 390, height: 844 },
  });
  const page = await context.newPage();
  try {
    const state = await fixture(page);
    await enter(page);
    await move(page, 2);
    await page.getByRole("button", { name: "Switch to dark mode" }).tap();
    await expect(page.locator("html")).toHaveClass(/theme-dark/);
    await expect(page.locator("html")).not.toHaveClass(/service-card-transition/);
    await page.evaluate(async () => {
      await Promise.all(
        document.getAnimations()
          .filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
          .map((animation) => animation.finished.catch(() => undefined)),
      );
    });
    const help = card(page).getByRole("button", {
      name: "How discovery order works",
      exact: true,
    });
    const tooltip = page.locator("[data-service-order-help]");
    const url = page.url();
    const draft = await card(page)
      .locator("[data-ordering-row]")
      .evaluateAll((rows) => rows.map((row) => row.getAttribute("data-ordering-row")));
    const search = page.getByRole("textbox", { name: "Search services and connections" });
    const query = await search.inputValue();
    await help.tap();
    await expect(help).toHaveAttribute("data-state", "instant-open");
    await expect(tooltip).toBeVisible();
    await expect(page.getByRole("tooltip").getByRole("listitem")).toHaveCount(3);
    await expect(tooltip.getByRole("link")).toHaveCount(0);
    await page.screenshot({
      path: testInfo.outputPath("info-touch-editor-390-dark.png"),
      animations: "disabled",
    });
    await help.tap();
    await expect(tooltip).toHaveCount(0);
    await expect(help).toHaveAttribute("data-state", "closed");
    await help.tap();
    await expect(help).toHaveAttribute("data-state", "instant-open");
    await expect(tooltip).toBeVisible();
    await card(page).getByText("4 enabled · 26 disabled", { exact: true }).tap();
    await expect(tooltip).toHaveCount(0);
    expect(
      await card(page)
        .locator("[data-ordering-row]")
        .evaluateAll((rows) => rows.map((row) => row.getAttribute("data-ordering-row"))),
    ).toEqual(draft);
    await expect(card(page).getByRole("button", { name: "Save", exact: true })).toBeEnabled();
    expect(await search.inputValue()).toBe(query);
    expect(page.url()).toBe(url);
    expect(state.writes).toEqual([]);
    expect(state.viewWrites).toEqual([]);
  } finally {
    await context.close();
  }
});
test("deferred identity recovery cannot write old draft and new actor can edit", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await move(page, 2);
  state.conflict = true;
  await card(page).getByRole("button", { name: "Save", exact: true }).click();
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let reached = false;
  await page.route("**/api/v1/service-preferences", async (route) => {
    reached = true;
    await gate;
    await route.fallback();
  });
  await card(page)
    .getByRole("button", { name: "Overwrite", exact: true })
    .click();
  await expect.poll(() => reached).toBe(true);
  await page.evaluate(async () => {
    const { useAuthStore } = await import("/src/stores/auth-store.ts");
    useAuthStore
      .getState()
      .setUser({ ...useAuthStore.getState().user, id: "another-human" });
  });
  release();
  await expect(card(page).getByRole("form")).toHaveCount(0);
  await expand(page);
  await expect(
    card(page).getByRole("button", { name: "Reorder discovery", exact: true }),
  ).toBeEnabled();
  await card(page)
    .getByRole("button", { name: "Reorder discovery", exact: true })
    .click();
  await move(page, 2);
  expect(state.writes).toHaveLength(1);
});
test("overview reuses inline rows, pills, info tooltip and pool metadata", async ({
  page,
}) => {
  await fixture(page);
  await page.goto(`/keys/services/${encodeURIComponent(group)}`);
  await expect(
    page.getByRole("heading", { name: "Anthropic", exact: true }),
  ).toBeVisible();
  await expect(row(page, 4).getByLabel(/Saved order position 2/)).toBeVisible();
  await page.getByRole("button", { name: "How discovery order works", exact: true }).hover();
  await expect(page.getByRole("tooltip")).toContainText("Connections that match equally are shown in your order.");
  await expect(page.locator("[data-service-order-help]").getByRole("link")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(
    row(page, 2).getByText("Explicit route · Priority 7"),
  ).toBeVisible();
  await page.getByRole("button", { name: "Reorder discovery", exact: true }).click();
  await move(page, 2);
  await expect(
    page.getByRole("form", { name: "Agent order for Anthropic" }),
  ).toBeAttached();
});
test("overview row History guards before changing connection or tab", async ({
  page,
}) => {
  const state = await fixture(page);
  await page.goto(`/keys/services/${encodeURIComponent(group)}`);
  await page.getByRole("button", { name: "Reorder discovery", exact: true }).click();
  await move(page, 2);
  const history = row(page, 2).getByRole("button", { name: /^History for/ });
  let prompts = 0;
  const dismiss = async (dialog: import("@playwright/test").Dialog) => {
    prompts++;
    await dialog.dismiss();
  };
  page.on("dialog", dismiss);
  await history.click();
  await expect(
    page.getByRole("tab", { name: "Connections", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("form", { name: "Agent order for Anthropic" }),
  ).toBeAttached();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeEnabled();
  expect(
    await page
      .locator("[data-ordering-row]")
      .evaluateAll((rows) =>
        rows.slice(0, 3).map((row) => row.getAttribute("data-ordering-row")),
      ),
  ).toEqual([ids[0], ids[2], ids[4]]);
  expect(prompts).toBe(1);
  page.off("dialog", dismiss);
  page.once("dialog", async (dialog) => {
    prompts++;
    await dialog.accept();
  });
  await history.click();
  await expect(
    page.getByRole("tab", { name: "History", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("combobox", { name: "Connection history" }),
  ).toContainText(service(2).label);
  await expect(
    page.getByRole("form", { name: "Agent order for Anthropic" }),
  ).toHaveCount(0);
  expect(prompts).toBe(2);
  expect(state.writes).toEqual([]);
  expect(state.viewWrites).toEqual([]);
});
test("table view Saved positions, two-member disabled eligibility, and single/custom absence", async ({
  page,
}) => {
  const state = await fixture(page, 2);
  state.disabled = [1];
  await page.reload();
  await enter(page);
  await expect(card(page).getByRole("form")).toBeAttached();
  await expect(card(page).getByText(/1 enabled · 1 disabled/)).toBeVisible();
  await expect(row(page, 1).getByLabel(/Saved order position 2/)).toBeVisible();
  await card(page).getByRole("button", { name: "Cancel", exact: true }).click();
  state.count = 1;
  await page.reload();
  await expand(page);
  await expect(
    card(page).getByRole("button", { name: "Reorder discovery", exact: true }),
  ).toHaveCount(0);
  await expect(
    card(page).getByRole("region", { name: /Agent discovery order for/ }),
  ).toHaveCount(0);
  state.count = 30;
  await page.reload();
  await page.getByRole("button", { name: "Table view", exact: true }).click();
  await expect(row(page, 4).getByLabel(/Saved order position 2/)).toHaveText(
    "Saved #2 · disabled",
  );
  await expect(
    page.getByRole("button", { name: "Reorder discovery", exact: true }),
  ).toHaveCount(0);
});

test("overview legacy provenance arrives during ordering and survives inventory-error recovery", async ({
  page,
}) => {
  const state = await fixture(page);
  state.legacy = true;
  let resume!: () => void;
  const gate = new Promise<void>((resolve) => {
    resume = resolve;
  });
  let reached = false;
  await page.route("**/api/v1/user-services", async (route) => {
    reached = true;
    await gate;
    await route.fallback();
  });
  await page.goto(`/keys/services/${encodeURIComponent(group)}`);
  await expect.poll(() => reached).toBe(true);
  await page.getByRole("button", { name: "Reorder discovery", exact: true }).click();
  await move(page, 2);
  const draft = await page
    .locator("[data-ordering-row]")
    .evaluateAll((rows) =>
      rows.map((row) => row.getAttribute("data-ordering-row")),
    );
  resume();
  await expect(
    row(page, 2).getByText(
      "A very long organization name for mobile wrapping",
      { exact: true },
    ),
  ).toBeVisible();
  expect(
    await page
      .locator("[data-ordering-row]")
      .evaluateAll((rows) =>
        rows.map((row) => row.getAttribute("data-ordering-row")),
      ),
  ).toEqual(draft);
  state.failInventory = true;
  await focusRefetch(page);
  await expect(
    page.getByText("Failed to refresh connections. Your edits are kept."),
  ).toBeVisible({ timeout: 15000 });
  await expect(
    page.getByRole("form", { name: "Agent order for Anthropic" }),
  ).toBeAttached();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  state.failInventory = false;
  await page.getByRole("button", { name: "Retry reads", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeEnabled();
  await expect(
    row(page, 2).getByText(
      "A very long organization name for mobile wrapping",
      { exact: true },
    ),
  ).toBeVisible();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("form")).toHaveCount(0);
  expect(state.writes[0]?.ordered).toEqual(draft);
  expect(state.writes[0]?.expected_version).toBe(1);
  expect(state.viewWrites).toEqual([]);
});

test("all group rows disappearing preserves an in-card Cancel action", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await move(page, 2);
  state.removed = ids.slice(0, 30);
  await focusRefetch(page);
  await expect(card(page).getByRole("form")).toBeAttached();
  await expect(card(page).locator("[data-ordering-row]")).toHaveCount(0);
  await card(page).getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(card(page)).toHaveCount(0);
  expect(state.writes).toEqual([]);
});

test("entire authorized inventory disappearing retains the draft until Cancel shows the empty state", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await move(page, 2);
  state.removed = ids.slice(0, 30);
  state.includeSlack = false;
  await focusRefetch(page);
  await expect(card(page).getByRole("form")).toBeAttached();
  await expect(card(page).locator("[data-ordering-row]")).toHaveCount(0);
  await expect(
    page.getByText("No AI services yet", { exact: true }),
  ).toHaveCount(0);
  await card(page).getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(card(page)).toHaveCount(0);
  await expect(
    page.getByText("No AI services yet", { exact: true }),
  ).toBeVisible();
  expect(state.writes).toEqual([]);
  expect(state.viewWrites).toEqual([]);
});

for (const surface of ["card", "overview"] as const) {
  for (const width of [390, 1024, 1440]) {
    test(`${surface} sticky Reorder discovery, Save and Cancel stay beside discovery at ${width}px`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize({ width, height: 844 });
      const state = await fixture(page);
      if (surface === "overview")
        await page.goto(`/keys/services/${encodeURIComponent(group)}`);
      else await expand(page);
      const scope = surface === "card" ? card(page) : page;
      await page.getByRole("button", { name: "Switch to dark mode" }).click();
      await expect(page.locator("html")).toHaveClass(/theme-dark/);
      await expect(page.locator("html")).not.toHaveClass(/service-card-transition/);
      await page.evaluate(async () => {
        await Promise.all(
          document.getAnimations()
            .filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
            .map((animation) => animation.finished.catch(() => undefined)),
        );
      });
      const bar = scope.locator("[data-service-order-actions]");
      const entry = bar.getByRole("button", {
        name: "Reorder discovery",
        exact: true,
      });
      await expect(entry).toHaveCount(1);
      await expect(entry).toBeEnabled();
      await expect(entry).toHaveClass(/nyx-gradient-vivid/);
      await assertOrderActionsNearDiscovery(bar);
      await page.locator("main").evaluate((element) => {
        element.scrollTop = 0;
      });
      if (surface === "card") {
        await bar.evaluate((element) => {
          const main = element.closest("main")!;
          main.scrollTop +=
            element.getBoundingClientRect().top -
            main.getBoundingClientRect().top -
            180;
        });
      }
      await expect(entry).toBeInViewport();
      await page.screenshot({
        path: testInfo.outputPath(`discovery-idle-${surface}-${width}.png`),
        animations: "disabled",
      });
      const idleHelp = bar.getByRole("button", { name: "How discovery order works", exact: true });
      await idleHelp.hover();
      await expect(page.locator("[data-service-order-help]")).toBeVisible();
      await page.screenshot({
        path: testInfo.outputPath(`info-idle-open-${surface}-${width}-dark.png`),
        animations: "disabled",
      });
      await page.keyboard.press("Escape");
      await expect(page.locator("[data-service-order-help]")).toHaveCount(0);
      if (surface === "overview") {
        const cover = await bar.evaluate((element) => ({
          top:
            element.getBoundingClientRect().top +
            parseFloat(getComputedStyle(element, "::before").top),
          metadataBottom: element.parentElement!.previousElementSibling!
            .getBoundingClientRect().bottom,
        }));
        expect(cover.top).toBeGreaterThanOrEqual(cover.metadataBottom);
      }
      if (surface === "card") {
        const chevron = bar
          .getByRole("button", { name: "Collapse Anthropic connections" })
          .locator("svg");
        await expect(chevron).toHaveCount(1);
        await expect(chevron).toHaveClass(/lucide-chevron-right/);
        const hide = bar.getByRole("button", { name: "Collapse Anthropic connections" });
        await expect(hide).toHaveAttribute("aria-expanded", "true");
        await expect(hide).toHaveClass(/bg-overlay text-foreground/);
        expect(await hide.evaluate((element) => getComputedStyle(element).backgroundColor)).not.toBe("rgba(0, 0, 0, 0)");
      }
      await entry.click();
      await expect(entry).toHaveCount(0);
      await expect(bar.getByText("Discovery order", { exact: true })).toBeVisible();
      const save = bar.getByRole("button", { name: "Save", exact: true });
      const cancel = bar.getByRole("button", { name: "Cancel", exact: true });
      await expect(
        scope.getByRole("button", { name: "Save", exact: true }),
      ).toHaveCount(1);
      await expect(
        scope.getByRole("button", { name: "Cancel", exact: true }),
      ).toHaveCount(1);
      await expect(save).toBeDisabled();
      await expect(cancel).toBeEnabled();
      await expect(save).toHaveClass(/nyx-gradient-vivid/);
      await assertOrderActionsNearDiscovery(bar);
      const help = bar.getByRole("button", { name: "How discovery order works", exact: true });
      const tooltip = page.locator("[data-service-order-help]");
      const form = scope.getByRole("form", {
        name: "Agent order for Anthropic",
      });
      expect(
        await save.evaluate((button) => (button as HTMLButtonElement).form?.id),
      ).toBe(await form.getAttribute("id"));
      await expect(form.locator("table")).toHaveCount(0);
      await move(page, 2);
      await expect(save).toBeEnabled();
      const main = page.locator("main");
      const scrollToMiddle = async () => {
        await row(page, 14).evaluate((element) => {
          const main = element.closest("main")!;
          main.scrollTop +=
            element.getBoundingClientRect().top -
            main.getBoundingClientRect().top -
            450;
        });
        await expect
          .poll(() => main.evaluate((element) => element.scrollTop))
          .toBeGreaterThan(400);
      };
      const assertVisibleActions = async () => {
        await assertOrderActionsNearDiscovery(bar);
        const mainBox = (await main.boundingBox())!;
        const boxes = await Promise.all([
          bar.boundingBox(),
          save.boundingBox(),
          cancel.boundingBox(),
          bar.getByText("Discovery order", { exact: true }).boundingBox(),
        ]);
        for (const box of boxes) {
          expect(box).not.toBeNull();
          expect(box!.y).toBeGreaterThanOrEqual(mainBox.y - 1);
          expect(box!.y + box!.height).toBeLessThanOrEqual(
            mainBox.y + mainBox.height,
          );
          expect(box!.x).toBeGreaterThanOrEqual(0);
          expect(box!.x + box!.width).toBeLessThanOrEqual(width);
        }
        const [, saveBox, cancelBox, labelBox] = boxes;
        for (const [a, b] of [
          [saveBox!, cancelBox!],
          [saveBox!, labelBox!],
          [cancelBox!, labelBox!],
        ]) {
          expect(
            a.x < b.x + b.width &&
              a.x + a.width > b.x &&
              a.y < b.y + b.height &&
              a.y + a.height > b.y,
          ).toBe(false);
        }
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        ).toBe(true);
      };
      await scrollToMiddle();
      await assertVisibleActions();
      const tableBoxBeforeHelp = (await row(page, 14).boundingBox())!;
      await help.focus();
      await expect(tooltip).toBeVisible();
      const tooltipBox = (await tooltip.boundingBox())!;
      expect(tooltipBox.x).toBeGreaterThanOrEqual(16);
      expect(tooltipBox.x + tooltipBox.width).toBeLessThanOrEqual(width - 16);
      expect(tooltipBox.y).toBeGreaterThanOrEqual(0);
      expect(tooltipBox.y + tooltipBox.height).toBeLessThanOrEqual(844);
      expect((await row(page, 14).boundingBox())!.y).toBeCloseTo(tableBoxBeforeHelp.y, 0);
      await page.screenshot({
        path: testInfo.outputPath(`info-edit-open-${surface}-${width}-dark.png`),
        animations: "disabled",
      });
      await page.keyboard.press("Escape");
      await expect(tooltip).toHaveCount(0);
      await expect(help).toBeFocused();
      if (surface === "overview") {
        const gutter = await bar.evaluate((element) => {
          const main = element.closest("main")!;
          const mainBox = main.getBoundingClientRect();
          const barBox = element.getBoundingClientRect();
          const cover = getComputedStyle(element, "::before");
          const inset = parseFloat(getComputedStyle(main).paddingTop);
          return {
            height: parseFloat(cover.height),
            top: barBox.top + parseFloat(cover.top),
            mainTop: mainBox.top,
            inset,
            covered: document.elementFromPoint(
              barBox.x + barBox.width / 2,
              mainBox.top + inset / 2,
            ) === element,
          };
        });
        expect(gutter.height).toBe(gutter.inset);
        expect(gutter.top).toBeCloseTo(gutter.mainTop, 0);
        expect(gutter.covered).toBe(true);
      }
      const beforeScroll = await main.evaluate((element) => element.scrollTop);
      await page.screenshot({
        path: testInfo.outputPath(`sticky-${surface}-${width}.png`),
        animations: "disabled",
      });
      const box = (await save.boundingBox())!;
      state.delayInventory = 250;
      state.delaySave = 400;
      // Click the measured visible control without Playwright scrolling it into view.
      await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
      await expect(save).toBeDisabled();
      await expect(cancel).toBeDisabled();
      await expect(form).toHaveCount(0);
      await expect(entry).toBeFocused();
      expect(state.writes).toHaveLength(1);
      expect(state.writes[0]?.group).toBe(group);
      expect(state.writes[0]?.ordered).toHaveLength(30);
      expect(
        Math.abs(
          (await main.evaluate((element) => element.scrollTop)) - beforeScroll,
        ),
      ).toBeLessThan(100);
      await entry.press("Enter");
      await move(page, 2);
      await scrollToMiddle();
      await assertVisibleActions();
      await cancel.focus();
      await cancel.press("Enter");
      await expect(form).toHaveCount(0);
      await expect(entry).toBeFocused();
      expect(state.writes).toHaveLength(1);
      expect(state.viewWrites).toEqual([]);
      await entry.press("Enter");
      await move(page, 2);
      await scrollToMiddle();
      await assertVisibleActions();
      state.failSave = true;
      const failedSave = (await save.boundingBox())!;
      await page.mouse.click(
        failedSave.x + failedSave.width / 2,
        failedSave.y + failedSave.height / 2,
      );
      const error = scope.getByRole("alert").filter({
        hasText: "Could not save agent order. Your edits are kept.",
      });
      await expect(error).toBeFocused();
      const errorBox = (await error.boundingBox())!;
      const barBox = (await bar.boundingBox())!;
      const mainBox = (await main.boundingBox())!;
      expect(errorBox.y).toBeGreaterThanOrEqual(barBox.y + barBox.height);
      expect(errorBox.y + errorBox.height).toBeLessThanOrEqual(
        mainBox.y + mainBox.height,
      );
      expect(
        await error.evaluate((element) => {
          const box = element.getBoundingClientRect();
          return element.contains(
            document.elementFromPoint(
              box.x + box.width / 2,
              box.y + box.height / 2,
            ),
          );
        }),
      ).toBe(true);
      await expect(scope.locator("[data-ordering-row]")).toHaveCount(30);
      expect(state.writes).toHaveLength(2);
      await scrollToMiddle();
      await assertVisibleActions();
      await save.focus();
      const repeatedSave = (await save.boundingBox())!;
      await page.mouse.click(
        repeatedSave.x + repeatedSave.width / 2,
        repeatedSave.y + repeatedSave.height / 2,
      );
      await expect.poll(() => state.writes.length).toBe(3);
      await expect(error).toBeFocused();
      await expect(error).toHaveCount(1);
      const repeatedErrorBox = (await error.boundingBox())!;
      const repeatedBarBox = (await bar.boundingBox())!;
      expect(repeatedErrorBox.y).toBeGreaterThanOrEqual(
        repeatedBarBox.y + repeatedBarBox.height,
      );
      expect(
        await error.evaluate((element) => {
          const bounds = element.getBoundingClientRect();
          return element.contains(
            document.elementFromPoint(
              bounds.x + bounds.width / 2,
              bounds.y + bounds.height / 2,
            ),
          );
        }),
      ).toBe(true);
      expect(state.viewWrites).toEqual([]);
    });
  }
}

test("unrelated access-panel Show all keys cannot submit a dirty order", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await move(page, 2);
  const draft = await card(page)
    .locator("[data-ordering-row]")
    .evaluateAll((rows) =>
      rows.map((row) => row.getAttribute("data-ordering-row")),
    );
  await row(page, 0)
    .getByRole("button", { name: "Agent key access for Anthropic" })
    .click();
  const panel = card(page).getByRole("region", {
    name: "Agent key access for Anthropic",
  });
  await expect(panel.getByRole("table").locator("tbody tr")).toHaveCount(3);
  await panel.getByRole("button", { name: "Show all 5 keys" }).click();
  await expect(panel.getByRole("table").locator("tbody tr")).toHaveCount(5);
  await panel.getByRole("button", { name: "Show fewer keys" }).click();
  await expect(panel.getByRole("table").locator("tbody tr")).toHaveCount(3);
  expect(state.writes).toEqual([]);
  await expect(
    card(page).getByRole("button", { name: "Save", exact: true }),
  ).toBeEnabled();
  expect(
    await card(page)
      .locator("[data-ordering-row]")
      .evaluateAll((rows) =>
        rows.map((row) => row.getAttribute("data-ordering-row")),
      ),
  ).toEqual(draft);
});

test("sticky Save brings one local 201-row validation error into view without a write", async ({
  page,
}) => {
  const state = await fixture(page, 201);
  await enter(page);
  await move(page, 2);
  await row(page, 14).evaluate((element) =>
    element.scrollIntoView({ block: "center" }),
  );
  const save = card(page)
    .locator("[data-service-order-actions]")
    .getByRole("button", { name: "Save", exact: true });
  const box = (await save.boundingBox())!;
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.y + box.height).toBeLessThanOrEqual(844);
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  const error = card(page)
    .getByRole("alert")
    .filter({ hasText: "At most 200 connections" });
  await expect(error).toHaveCount(1);
  await expect(error).toBeFocused();
  await expect(error).toBeInViewport();
  const errorBox = (await error.boundingBox())!;
  const actionBox = (await card(page)
    .locator("[data-service-order-actions]")
    .boundingBox())!;
  const mainBox = (await page.locator("main").boundingBox())!;
  expect(errorBox.y).toBeGreaterThanOrEqual(actionBox.y + actionBox.height);
  expect(errorBox.y + errorBox.height).toBeLessThanOrEqual(
    mainBox.y + mainBox.height,
  );
  expect(
    await error.evaluate((element) => {
      const box = element.getBoundingClientRect();
      return element.contains(
        document.elementFromPoint(
          box.x + box.width / 2,
          box.y + box.height / 2,
        ),
      );
    }),
  ).toBe(true);
  expect(state.writes).toEqual([]);
  await row(page, 14).evaluate((element) =>
    element.scrollIntoView({ block: "center" }),
  );
  await save.focus();
  const repeatedSave = (await save.boundingBox())!;
  await page.mouse.click(
    repeatedSave.x + repeatedSave.width / 2,
    repeatedSave.y + repeatedSave.height / 2,
  );
  await expect(error).toBeFocused();
  const repeatedErrorBox = (await error.boundingBox())!;
  const repeatedActionBox = (await card(page)
    .locator("[data-service-order-actions]")
    .boundingBox())!;
  expect(repeatedErrorBox.y).toBeGreaterThanOrEqual(
    repeatedActionBox.y + repeatedActionBox.height,
  );
  expect(state.writes).toEqual([]);
  await expect(card(page).locator("[data-ordering-row]")).toHaveCount(201);
  const reset = card(page).getByRole("button", {
    name: "Reset to default",
    exact: true,
  });
  await card(page).evaluate((element) => {
    const main = element.closest("main")!;
    const cover = element.querySelector("[data-service-order-actions]")!;
    const reset = Array.from(element.querySelectorAll("button")).find(
      (button) => button.textContent === "Reset to default",
    )!;
    main.scrollTop +=
      reset.getBoundingClientRect().top -
      cover.getBoundingClientRect().bottom -
      16;
  });
  await expect
    .poll(() =>
      card(page).evaluate((element) => {
        const main = element.closest("main")!.getBoundingClientRect();
        const cover = element
          .querySelector("[data-service-order-actions]")!
          .getBoundingClientRect();
        const reset = Array.from(element.querySelectorAll("button")).find(
          (button) => button.textContent === "Reset to default",
        )!;
        const bounds = reset.getBoundingClientRect();
        return (
          bounds.top >= cover.bottom &&
          bounds.bottom <= main.bottom &&
          reset.contains(
            document.elementFromPoint(
              bounds.x + bounds.width / 2,
              bounds.y + bounds.height / 2,
            ),
          )
        );
      }),
    )
    .toBe(true);
  const resetBox = (await reset.boundingBox())!;
  page.once("dialog", (dialog) => dialog.accept());
  await page.mouse.click(
    resetBox.x + resetBox.width / 2,
    resetBox.y + resetBox.height / 2,
  );
  await expect(error).toHaveCount(0);
  expect(state.writes).toEqual([]);
  expect(
    await card(page)
      .locator("[data-ordering-row]")
      .evaluateAll((rows) =>
        rows.map((row) => row.getAttribute("data-ordering-row")),
      ),
  ).toEqual(ids.slice(0, 201));
  await expect(save).toBeEnabled();
  const resetSaveBox = (await save.boundingBox())!;
  expect(
    await save.evaluate((element) => {
      const bounds = element.getBoundingClientRect();
      return element.contains(
        document.elementFromPoint(
          bounds.x + bounds.width / 2,
          bounds.y + bounds.height / 2,
        ),
      );
    }),
  ).toBe(true);
  await page.mouse.click(
    resetSaveBox.x + resetSaveBox.width / 2,
    resetSaveBox.y + resetSaveBox.height / 2,
  );
  await expect(card(page).getByRole("form")).toHaveCount(0);
  expect(state.writes).toEqual([{ group, ordered: [], expected_version: 1 }]);
});
