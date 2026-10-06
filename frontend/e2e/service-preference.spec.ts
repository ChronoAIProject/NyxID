import { expect, test, type Page } from "@playwright/test";

const ids = [
  "11111111-1111-4111-8111-111111111111",
  "22222222-2222-4222-8222-222222222222",
  "33333333-3333-4333-8333-333333333333",
];
const labels = ["Alpha", "Beta", "Gamma"];
function service(index: number) {
  return {
    id: ids[index],
    label: labels[index],
    slug: labels[index]?.toLowerCase(),
    endpoint_url: "https://example.com",
    endpoint_id: `endpoint-${index}`,
    credential_type: "bearer",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: "active",
    catalog_service_id: null,
    catalog_service_slug: null,
    catalog_service_name: null,
    node_id: null,
    node_priority: 0,
    is_active: true,
    ws_frame_injections: [],
    auto_connected: index === 1,
    expires_at: null,
    last_used_at: null,
    error_message: null,
    created_at: "2026-10-07T00:00:00Z",
    service_type: "http",
    ssh_host: null,
    ssh_port: null,
    ssh_ca_public_key: null,
    ssh_allowed_principals: null,
    ssh_certificate_ttl_minutes: null,
    credential_source:
      index === 2
        ? {
            type: "org",
            org_id: "44444444-4444-4444-8444-444444444444",
            org_name: "A very long organization name that wraps on mobile",
            role: "admin",
            allowed: true,
          }
        : { type: "personal" },
  };
}
async function fixture(page: Page, count = 3, longLabels = false) {
  const state = {
    ordered: [ids[0]!, ids[1]!],
    version: 1,
    failRead: false,
    failInventory: false,
    failSave: false,
    conflict: false,
    gone: false,
    count,
    longLabels,
    deletedIds: [] as string[],
    disabledIds: [] as string[],
    grouped: false,
    withPool: false,
    legacySource: false,
    viewWrites: [] as unknown[],
    writes: [] as { ordered: string[]; expected_version: number }[],
  };
  await page.route("**/api/v1/**", async (route) => {
    const req = route.request();
    const path = new URL(req.url()).pathname;
    let status = 200;
    let body: unknown = {};
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
    } else if (path === "/api/v1/service-insights") body = { connections: [] };
    else if (
      path === "/api/v1/service-pools" ||
      path.endsWith("/service-pools")
    )
      body = {
        pools:
          state.withPool && !new URL(req.url()).searchParams.has("org_id")
            ? [
                {
                  id: "pool",
                  user_id: "human",
                  name: "Example route",
                  slug: "example-route",
                  strategy: "priority",
                  members: [
                    {
                      user_service_id: ids[0],
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
              ]
            : [],
      };
    else if (path === "/api/v1/public/config")
      body = { telemetry_dsn: null, telemetry_share_analytics: false };
    else if (path === "/api/v1/service-preferences") {
      if (req.method() === "PUT") {
        const write = req.postDataJSON() as (typeof state.writes)[number];
        state.writes.push(write);
        if (state.conflict) {
          state.conflict = false;
          state.version += 1;
          status = 409;
        } else if (write.ordered.some((id) => state.deletedIds.includes(id)))
          status = 400;
        else if (state.failSave) status = 503;
        else if (write.expected_version !== state.version) status = 409;
        else {
          state.ordered = write.ordered;
          state.version += 1;
        }
      } else if (state.failRead) status = 503;
      else if (state.gone) status = 404;
      body =
        status === 200
          ? {
              ordered: state.ordered.filter(
                (id) => !state.deletedIds.includes(id),
              ),
              version: state.version,
              updated_at: null,
            }
          : {
              error: "fixture",
              error_code: status === 409 ? 1004 : 1000,
              message: "Fixture failure",
            };
    } else if (path === "/api/v1/keys") {
      status = state.failInventory ? 503 : 200;
      body = state.failInventory
        ? { message: "Inventory unavailable", error_code: 1000 }
        : {
            keys: Array.from({ length: state.count }, (_, i) => {
              const row = {
                ...service(i),
                is_active: !state.disabledIds.includes(ids[i]!),
              };
              if (state.legacySource && i === 2)
                Reflect.deleteProperty(row, "credential_source");
              if (state.grouped)
                Object.assign(row, {
                  catalog_service_id: "example-api",
                  catalog_service_slug: "example-api",
                  catalog_service_name: "Example API",
                });
              if (state.longLabels && i === 2)
                row.label =
                  "Gamma with a very long service label that wraps on a narrow mobile screen";
              const rank = state.ordered
                .filter((id) => !state.deletedIds.includes(id))
                .indexOf(row.id!);
              return { ...row, preference_rank: rank < 0 ? null : rank + 1 };
            }).filter((row) => !state.deletedIds.includes(row.id!)),
          };
    } else if (path === "/api/v1/user-services")
      body = {
        services: state.legacySource
          ? [{ id: ids[2], credential_source: service(2).credential_source }]
          : [],
      };
    else if (path === "/api/v1/nodes") body = { nodes: [] };
    else if (path === "/api/v1/orgs") body = { orgs: [] };
    else if (path === "/api/v1/catalog") body = { entries: [] };
    else {
      status = 404;
      body = { message: "Not found", error_code: 1000 };
    }
    await route.fulfill({ status, json: body });
  });
  await page.goto("/keys");
  await expect(
    page.getByRole("heading", { name: "Services & Credentials" }),
  ).toBeVisible();
  return state;
}
async function enter(page: Page) {
  await page.getByRole("button", { name: "Reorder", exact: true }).click();
  await expect(
    page.getByRole("form", { name: "Service preference order" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /^Drag / }).first(),
  ).toBeEnabled();
}
async function drag(page: Page, from: string, to: string) {
  await page
    .getByRole("button", { name: `Drag ${from}`, exact: true })
    .scrollIntoViewIfNeeded();
  const a = await page
    .getByRole("button", { name: `Drag ${from}`, exact: true })
    .boundingBox();
  const b = await page
    .getByRole("button", { name: `Drag ${to}`, exact: true })
    .boundingBox();
  if (!a || !b) throw new Error("Missing drag handles");
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  await page.mouse.move(a.x + a.width / 2 + 8, a.y + a.height / 2, {
    steps: 2,
  });
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 15 });
  await page.mouse.up();
}

test("mouse order, grid/table saved pills, full inventory, focus and reload persistence", async ({
  page,
}) => {
  const state = await fixture(page);
  await expect(
    page.getByRole("button", {
      name: "Discovery preference 1 · Alpha",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByLabel("Discovery preference 2", { exact: true }),
  ).toHaveCount(0);
  await enter(page);
  await expect(
    page.getByText(
      "Editing your complete inventory · 3 connections. Filters and saved views are not applied here and are not changed.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Service filters" }),
  ).toHaveCount(0);
  await expect(page.getByText("Auto-connected", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Drag Alpha", exact: true }),
  ).toBeFocused();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await drag(page, "Gamma", "Alpha");
  await expect(
    page
      .locator(`[data-preference-item="${ids[2]}"]`)
      .getByLabel("Discovery preference 1", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Reorder", exact: true }),
  ).toBeFocused();
  expect(state.writes[0]).toEqual({
    ordered: [ids[2], ids[0], ids[1]],
    expected_version: 1,
  });
  await page.reload();
  await expect(
    page.getByRole("button", {
      name: "Discovery preference 1 · Gamma",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", {
      name: "Discovery preference 2 · Alpha",
      exact: true,
    }),
  ).toBeVisible();
  expect(state.viewWrites).toEqual([]);
  await page.getByRole("button", { name: /table view/i }).click();
  await expect(
    page.getByLabel("Discovery preference 1", { exact: true }),
  ).toBeVisible();
  await enter(page);
  await expect(page.locator("form a")).toHaveCount(0);
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
});

test("group chip uses complete inventory, pills preserve order and filters, editor leaves saved views unchanged", async ({
  page,
}) => {
  const state = await fixture(page);
  state.grouped = true;
  state.withPool = true;
  state.ordered = [ids[1]!, ids[2]!, ids[0]!];
  await page.reload();
  const group = page.getByRole("region", { name: "Example API", exact: true });
  const chip = group.getByRole("button", {
    name: "Discovery preference 1 · Beta",
    exact: true,
  });
  await expect(chip).toHaveText("Discovery #1");
  await expect(chip).toHaveAttribute(
    "title",
    "#3 · Alpha\n#1 · Beta\n#2 · Gamma",
  );
  await chip.click();
  const table = group.getByRole("table");
  await expect(
    table.getByRole("link", { name: /^View .+ connection details/ }),
  ).toHaveText(["Alpha", "Gamma"]);
  const alpha = table.getByRole("link", {
    name: "View Alpha connection details (Personal)",
    exact: true,
  });
  await expect(alpha.locator("xpath=following-sibling::*[1]")).toHaveAttribute(
    "aria-label",
    "Discovery preference 3",
  );
  await expect(alpha.locator("xpath=following-sibling::*[2]")).toContainText(
    "Credential check needed",
  );
  await expect(alpha.locator("xpath=ancestor::tr")).toContainText(
    "Example route · Priority 7",
  );
  await expect(
    table.getByLabel("Discovery preference 2", { exact: true }),
  ).toHaveText("Discovery #2");

  await page.getByRole("button", { name: "Organization", exact: true }).click();
  await page
    .getByRole("checkbox", {
      name: "A very long organization name that wraps on mobile",
      exact: true,
    })
    .check();
  await page.keyboard.press("Escape");
  await expect(
    table.getByLabel("Discovery preference 2", { exact: true }),
  ).toHaveText("Discovery #2");
  await expect(
    table.getByLabel("Discovery preference 3", { exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Clear filters", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Service view: All services", exact: true })
    .click();
  await page.getByRole("button", { name: "Service", exact: true }).click();
  await page
    .getByRole("checkbox", { name: "Example API", exact: true })
    .check();
  await page.keyboard.press("Escape");
  const search = page.getByRole("textbox", {
    name: "Search services and connections",
    exact: true,
  });
  await search.fill("Alpha");
  await search.press("Enter");
  await expect(
    table.getByRole("link", { name: /^View .+ connection details/ }),
  ).toHaveText(["Alpha"]);
  const filters = page.getByRole("region", { name: "Service filters" });
  const before = await filters.innerText();
  for (const outcome of ["Cancel", "Save"] as const) {
    await enter(page);
    await expect(
      page.getByText(
        "Editing your complete inventory · 3 connections. Filters and saved views are not applied here and are not changed.",
      ),
    ).toBeVisible();
    await expect(page.locator("[data-preference-item]")).toHaveCount(4);
    await expect(
      page.getByRole("button", { name: "Drag Gamma", exact: true }),
    ).toBeVisible();
    await expect(page.locator("form a")).toHaveCount(0);
    await expect(filters).toHaveCount(0);
    if (outcome === "Save")
      await page
        .getByRole("button", { name: "Unrank Beta", exact: true })
        .click();
    await page.getByRole("button", { name: outcome, exact: true }).click();
    await expect(filters).toBeVisible();
    await expect.poll(() => filters.innerText()).toBe(before);
    await expect(
      group.getByRole("button", {
        name: "Collapse Example API connections",
        exact: true,
      }),
    ).toBeVisible();
    expect(state.viewWrites).toEqual([]);
  }
  await group
    .getByRole("link", {
      name: "View all Example API service details",
      exact: true,
    })
    .click();
  await expect(
    page.getByRole("heading", { name: "Example API", exact: true }),
  ).toBeVisible();
  const overview = page.getByRole("table");
  await expect(
    overview.getByRole("link", { name: /^View .+ connection details/ }),
  ).toHaveText(["Alpha", "Beta", "Gamma"]);
  await expect(
    overview.getByLabel("Discovery preference 2", { exact: true }),
  ).toHaveText("Discovery #2");
  await expect(
    overview.getByLabel("Discovery preference 1", { exact: true }),
  ).toHaveText("Discovery #1");
  await expect(
    overview
      .getByRole("link", {
        name: "View Alpha connection details (Personal)",
        exact: true,
      })
      .locator("xpath=following-sibling::*[1]"),
  ).toHaveAttribute("aria-label", "Discovery preference 2");
  await page.goto("/keys?view=routing");
  await expect(
    page.getByRole("button", { name: "Reorder", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", {
      name: "Discovery preference 1 · Gamma",
      exact: true,
    })
    .click();
  await expect(
    page
      .getByRole("table")
      .getByLabel("Discovery preference 2", { exact: true }),
  ).toBeVisible();
  expect(state.viewWrites).toEqual([]);
});

test("keyboard drag, escape cancellation, divider and dirty view/tab confirmation", async ({
  page,
}) => {
  await fixture(page);
  await enter(page);
  const handle = page.getByRole("button", { name: "Drag Beta", exact: true });
  await expect
    .poll(() =>
      page
        .locator("[data-preference-item]")
        .evaluateAll((items) =>
          items.every((item) =>
            item
              .getAnimations()
              .every((animation) => animation.playState !== "running"),
          ),
        ),
    )
    .toBe(true);
  await handle.focus();
  await page.keyboard.press("Space");
  await expect(handle).toHaveAttribute("aria-pressed", "true");
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Escape");
  await expect(handle).not.toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await expect
    .poll(() =>
      page
        .locator("[data-preference-item]")
        .evaluateAll((items) =>
          items.every((item) =>
            item
              .getAnimations()
              .every((animation) => animation.playState !== "running"),
          ),
        ),
    )
    .toBe(true);
  await handle.focus();
  await page.keyboard.press("Space");
  await expect(handle).toHaveAttribute("aria-pressed", "true");
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  await page.keyboard.press("ArrowUp");
  await expect(page.locator('[role="status"][aria-live]')).toContainText(
    "Moved to position 1",
  );
  await page.keyboard.press("Space");
  await expect(
    page
      .locator(`[data-preference-item="${ids[1]}"]`)
      .getByLabel("Discovery preference 1", { exact: true }),
  ).toBeVisible();
  await expect(handle).toBeFocused();
  await expect(page.locator('[role="status"][aria-live]')).toHaveCount(1);
  page.once("dialog", (dialog) => dialog.dismiss());
  await page.getByRole("button", { name: /table view/i }).click();
  await expect(
    page.getByRole("form", { name: "Service preference order" }),
  ).toBeVisible();
  page.once("dialog", (dialog) => dialog.dismiss());
  await page.getByRole("tab", { name: "Agent Keys" }).click();
  await expect(
    page.getByRole("form", { name: "Service preference order" }),
  ).toBeVisible();
  await drag(page, "Unranked divider", "Beta");
  await expect(
    page.locator('[data-preference-item] [aria-label^="Discovery preference"]'),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
});

for (const outcome of ["Save", "Cancel"] as const) {
  test(`${outcome} restores focus after the closing inventory refetch becomes available`, async ({
    page,
  }) => {
    await fixture(page);
    await enter(page);
    if (outcome === "Save")
      await page
        .getByRole("button", { name: "Rank Gamma", exact: true })
        .click();
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    let held = false;
    let reads = 0;
    await page.route("**/api/v1/keys", async (route) => {
      reads += 1;
      // Save invalidation completes before close; hold the new list observer's read.
      if (reads === (outcome === "Save" ? 2 : 1)) {
        held = true;
        await gate;
      }
      await route.fallback();
    });
    await page.getByRole("button", { name: outcome, exact: true }).click();
    await expect.poll(() => held).toBe(true);
    await expect(
      page.getByRole("form", { name: "Service preference order" }),
    ).toHaveCount(0);
    const reorder = page.getByRole("button", { name: "Reorder", exact: true });
    await expect(reorder).toBeDisabled();
    await expect(reorder).not.toBeFocused();
    release();
    await expect(reorder).toBeEnabled();
    await expect(reorder).toBeFocused();
  });
}

test("save failure retains edits, conflict overwrite refetches version and persists", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await page.getByRole("button", { name: "Rank Gamma", exact: true }).click();
  state.failSave = true;
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByText("Could not save preference order. Your edits are kept."),
  ).toBeVisible();
  await expect(
    page
      .locator(`[data-preference-item="${ids[2]}"]`)
      .getByLabel("Discovery preference 3", { exact: true }),
  ).toBeVisible();
  state.failSave = false;
  state.conflict = true;
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Overwrite", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Overwrite", exact: true }).click();
  await expect(
    page.getByRole("form", { name: "Service preference order" }),
  ).toHaveCount(0);
  expect(state.writes.at(-1)).toEqual({ ordered: ids, expected_version: 2 });
  await page.reload();
  await enter(page);
  await expect(
    page
      .locator(`[data-preference-item="${ids[2]}"]`)
      .getByLabel("Discovery preference 3", { exact: true }),
  ).toBeVisible();
});

test("read failure fails closed, Retry, single service, compatibility and empty state", async ({
  page,
}) => {
  const state = await fixture(page, 1);
  state.failRead = true;
  await page.reload();
  await expect(
    page.getByText("Could not load preference order. Editing is unavailable."),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Reorder", exact: true }),
  ).toBeDisabled();
  expect(state.writes).toHaveLength(0);
  state.failRead = false;
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await enter(page);
  await expect(
    page.getByText(
      "Editing your complete inventory · 1 connection. Filters and saved views are not applied here and are not changed.",
    ),
  ).toBeVisible();
  await page.getByRole("button", { name: "Unrank Alpha", exact: true }).click();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect.poll(() => state.writes.at(-1)?.ordered).toEqual([]);
  state.gone = true;
  await page.reload();
  await expect(
    page.getByRole("button", { name: "Reorder", exact: true }),
  ).toHaveCount(0);
  state.gone = false;
  state.count = 0;
  await page.reload();
  await expect(
    page.getByRole("button", { name: "Reorder", exact: true }),
  ).toBeDisabled();
});

test("real touch drag and long mobile labels without overflow", async ({
  browser,
  baseURL,
}) => {
  const context = await browser.newContext({
    baseURL,
    hasTouch: true,
    isMobile: true,
    viewport: { width: 390, height: 844 },
  });
  const page = await context.newPage();
  await fixture(page, 3, true);
  await enter(page);
  await page
    .getByRole("button", { name: /^Drag Gamma/ })
    .scrollIntoViewIfNeeded();
  const session = await context.newCDPSession(page);
  const a = await page
    .getByRole("button", { name: /^Drag Gamma/ })
    .boundingBox();
  const b = await page
    .getByRole("button", { name: "Drag Alpha", exact: true })
    .boundingBox();
  if (!a || !b) throw new Error("Missing handles");
  const start = { x: a.x + a.width / 2, y: a.y + a.height / 2 };
  await session.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [start],
  });
  for (let i = 1; i <= 15; i++)
    await session.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [
        { x: start.x, y: start.y + ((b.y + b.height / 2 - start.y) * i) / 15 },
      ],
    });
  await session.send("Input.dispatchTouchEvent", {
    type: "touchEnd",
    touchPoints: [],
  });
  await expect(
    page
      .locator(`[data-preference-item="${ids[2]}"]`)
      .getByLabel("Discovery preference 1", { exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await context.close();
});

test("conflict reload clears dirty and inventory recovery preserves edits", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await page.getByRole("button", { name: "Rank Gamma", exact: true }).click();
  state.conflict = true;
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await page.getByRole("button", { name: "Reload order", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Rank Gamma", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Rank Gamma", exact: true }).click();
  state.conflict = true;
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Reload order", exact: true }),
  ).toBeVisible();
  state.failInventory = true;
  await page.getByRole("button", { name: "Reload order", exact: true }).click();
  await expect(
    page.getByText("Could not load services. Your preference edits are kept."),
  ).toBeVisible({ timeout: 15_000 });
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await expect(
    page
      .locator(`[data-preference-item="${ids[2]}"]`)
      .getByLabel("Discovery preference 3", { exact: true }),
  ).toBeVisible();
  state.failInventory = false;
  await page
    .getByText("Could not load services. Your preference edits are kept.")
    .locator("..")
    .getByRole("button", { name: "Retry", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Rank Alpha", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Reload order", exact: true }),
  ).toBeEnabled();
});

test("deferred conflict recovery cannot write as a switched account or resurrect a draft", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await page.getByRole("button", { name: "Rank Gamma", exact: true }).click();
  state.conflict = true;
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Overwrite", exact: true }),
  ).toBeVisible();
  let release: () => void = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let reached = false;
  await page.route("**/api/v1/service-preferences", async (route) => {
    if (route.request().method() === "GET") {
      reached = true;
      await gate;
    }
    await route.fallback();
  });
  await page.getByRole("button", { name: "Overwrite", exact: true }).click();
  await expect.poll(() => reached).toBe(true);
  await page.evaluate(async () => {
    const { useAuthStore } = await import("/src/stores/auth-store.ts");
    const user = useAuthStore.getState().user;
    useAuthStore.getState().setUser({ ...user, id: "another-human" });
  });
  await expect(
    page.getByRole("form", { name: "Service preference order" }),
  ).toHaveCount(0);
  release();
  await expect(
    page.getByRole("button", { name: "Reorder", exact: true }),
  ).toBeEnabled();
  await page.evaluate(async () => {
    const { useAuthStore } = await import("/src/stores/auth-store.ts");
    useAuthStore
      .getState()
      .setUser({ ...useAuthStore.getState().user, id: "human" });
  });
  await expect(
    page.getByRole("button", { name: "Reorder", exact: true }),
  ).toBeEnabled();
  await expect(
    page.getByRole("form", { name: "Service preference order" }),
  ).toHaveCount(0);
  expect(state.writes).toHaveLength(1);
});

test("editor screenshots for PM", async ({ page }, testInfo) => {
  const state = await fixture(page, 3, true);
  state.disabledIds = [ids[2]!];
  await page.reload();
  await enter(page);
  await page.screenshot({
    path: testInfo.outputPath("desktop-editor.png"),
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: testInfo.outputPath("mobile-editor.png"),
    fullPage: true,
  });
  await page
    .getByRole("button", { name: /^Drag Gamma/ })
    .scrollIntoViewIfNeeded();
  await expect(
    page
      .locator(`[data-preference-item="${ids[2]}"]`)
      .getByText("Disabled", { exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("mobile-editor-details.png"),
    fullPage: true,
  });
  await testInfo.attach("mobile editor details", {
    path: testInfo.outputPath("mobile-editor-details.png"),
    contentType: "image/png",
  });
  await testInfo.attach("desktop editor", {
    path: testInfo.outputPath("desktop-editor.png"),
    contentType: "image/png",
  });
  await testInfo.attach("mobile editor", {
    path: testInfo.outputPath("mobile-editor.png"),
    contentType: "image/png",
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});

test("actual stale-ID 400 refreshes inventory before dropping IDs and permits review/save", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await page.getByRole("button", { name: "Rank Gamma", exact: true }).click();
  state.deletedIds = [ids[1]!];
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByText(
      "Some services are no longer available. Refresh services before saving again.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Unrank Beta", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(page.locator(`[data-preference-item="${ids[1]}"]`)).toHaveCount(
    0,
  );
  await expect(
    page
      .locator(`[data-preference-item="${ids[2]}"]`)
      .getByLabel("Discovery preference 2", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect
    .poll(() => state.writes.at(-1)?.ordered)
    .toEqual([ids[0], ids[2]]);
  await expect(
    page.getByRole("form", { name: "Service preference order" }),
  ).toHaveCount(0);
});

test("conflict recovery remains available after preference refetch fails", async ({
  page,
}) => {
  const state = await fixture(page);
  await enter(page);
  await page.getByRole("button", { name: "Rank Gamma", exact: true }).click();
  state.conflict = true;
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Overwrite", exact: true }),
  ).toBeVisible();
  state.failRead = true;
  await page.getByRole("button", { name: "Overwrite", exact: true }).click();
  await expect(
    page.getByText(
      "Could not reload the current order. Your edits are kept. Retry recovery.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Overwrite", exact: true }),
  ).toBeEnabled();
  expect(state.writes).toHaveLength(1);
  state.failRead = false;
  await page.getByRole("button", { name: "Overwrite", exact: true }).click();
  await expect(
    page.getByRole("form", { name: "Service preference order" }),
  ).toHaveCount(0);
  expect(state.writes.at(-1)).toEqual({ ordered: ids, expected_version: 2 });
});

test("legacy org provenance joins late and survives stale inventory refresh without resetting order", async ({
  page,
}) => {
  const state = await fixture(page);
  state.legacySource = true;
  let release: () => void = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/v1/user-services", async (route) => {
    await gate;
    await route.fallback();
  });
  await page.reload();
  await enter(page);
  await page.getByRole("button", { name: "Rank Gamma", exact: true }).click();
  const gamma = page.locator(`[data-preference-item="${ids[2]}"]`);
  release();
  await expect(
    gamma.getByText("Org: A very long organization name that wraps on mobile", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    gamma.getByLabel("Discovery preference 3", { exact: true }),
  ).toHaveText("Discovery #3");
  expect(state.writes).toEqual([]);
  state.deletedIds.push(ids[0]!);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Drag Alpha", exact: true }),
  ).toHaveCount(0);
  await expect(
    gamma.getByText("Org: A very long organization name that wraps on mobile", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    gamma.getByLabel("Discovery preference 2", { exact: true }),
  ).toHaveText("Discovery #2");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect
    .poll(() => state.writes.at(-1))
    .toEqual({ ordered: [ids[1], ids[2]], expected_version: 1 });
});
