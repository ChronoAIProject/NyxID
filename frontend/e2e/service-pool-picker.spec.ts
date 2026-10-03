import { expect, test, type Page } from "@playwright/test";

const ids = [
  "11111111-1111-4111-8111-111111111111",
  "22222222-2222-4222-8222-222222222222",
  "33333333-3333-4333-8333-333333333333",
];
const candidates = ids.map((id, index) => ({
  user_service_id: id,
  name: index === 2 ? "Failed connection" : `Local pool member ${index + 1}`,
  slug: index === 2 ? "failed-connection" : `local-pool-member-${index + 1}`,
  is_active: true,
  eligible: false,
  reason:
    index === 2
      ? "credential_unavailable"
      : "compatibility_declaration_required",
  credential_binding: index === 2 ? "user" : "none",
  protocol: null,
  catalog_service_id: index === 2 ? "catalog-failed" : "catalog-shared",
  group_name: "Shared service",
  group_slug: index === 2 ? "failed-service" : "shared-service",
  requires_compatibility_declaration: true,
  cooldown_until: null,
  consecutive_failures: 0,
  last_status: null,
}));
const manyCandidates = Array.from({ length: 12 }, (_, index) => ({
  ...candidates[0]!,
  user_service_id: `44444444-4444-4444-8444-${String(index + 1).padStart(12, "0")}`,
  name: `Responsive member ${index + 1}`,
  slug: `responsive-member-${index + 1}`,
  catalog_service_id: `catalog-${index % 3}`,
  group_name: `Service group ${(index % 3) + 1}`,
  group_slug: `service-group-${(index % 3) + 1}`,
  eligible: true,
  reason: null,
  requires_compatibility_declaration: false,
}));

async function mockConnections(page: Page, sourceRows = candidates) {
  await page.route("**/api/v1/**", async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname.replace("/api/v1", "");
    let body: unknown = {};
    if (path === "/users/me") {
      body = {
        id: "44444444-4444-4444-8444-444444444444",
        email: "pool-layout@example.test",
        display_name: "Local review",
        role: "user",
        is_active: true,
        email_verified: true,
        created_at: "2026-10-01T00:00:00Z",
      };
    } else if (path === "/orgs") body = { orgs: [], organizations: [] };
    else if (path === "/catalog") body = { entries: [] };
    else if (path === "/keys" || path === "/api-keys") body = { keys: [] };
    else if (path === "/service-pools") body = { pools: [] };
    else if (path.endsWith("/candidates")) {
      const search = (url.searchParams.get("search") ?? "").toLowerCase();
      const peers = (url.searchParams.get("peer_ids") ?? "").split(",");
      body = {
        candidates: sourceRows.filter((row) =>
          url.searchParams.get("selected_only") === "true"
            ? peers.includes(row.user_service_id)
            : row.name.toLowerCase().includes(search) ||
              row.slug.includes(search) ||
              row.group_name.toLowerCase().includes(search) ||
              row.group_slug.includes(search),
        ),
        has_more: false,
        next_cursor: null,
        operation_checked: false,
        method: null,
        path: null,
      };
    } else if (path === "/runtime-config") {
      body = {
        release_integrity: {
          enabled: false,
          manifest_url: null,
          verification_ttl_secs: 300,
        },
      };
    } else if (path === "/nodes") body = { nodes: [] };
    else if (path === "/providers") body = { providers: [] };
    else if (path === "/services") body = { services: [] };
    await route.fulfill({ json: body });
  });
}

test("pool multi-select survives responsive resize and changing member cards without observer errors", async ({
  page,
}, testInfo) => {
  const runtimeErrors: string[] = [];
  page.on("pageerror", (error) => runtimeErrors.push(error.message));
  page.on("console", (entry) => {
    if (entry.type() === "error") runtimeErrors.push(entry.text());
  });
  // Vite's overlay may stop propagation before Playwright receives pageerror.
  // Record browser errors in capture phase too; never suppress the event.
  await page.addInitScript(() => {
    const errors: string[] = [];
    Object.assign(window, { poolLayoutErrors: errors });
    window.addEventListener(
      "error",
      (event) => {
        if (event.message) errors.push(event.message);
      },
      true,
    );
  });
  await mockConnections(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/keys?tab=pools");
  await page.getByRole("button", { name: /^Create pool$/i }).click();
  const form = page.getByRole("dialog", {
    name: "Create service pool",
    exact: true,
  });
  await form.getByLabel("Name", { exact: true }).fill("Responsive pool");
  const trigger = form.getByRole("button", {
    name: "Choose connections",
    exact: true,
  });
  await trigger.click();
  const picker = page.getByRole("dialog", {
    name: "Choose pool connections",
    exact: true,
  });
  const search = picker.getByRole("combobox", {
    name: "Search candidate services",
  });
  await expect(
    picker.getByRole("option", { name: "Failed connection", exact: true }),
  ).toHaveAttribute("aria-disabled", "true");
  await expect(
    picker.getByRole("group", {
      name: "Shared service (shared-service) (2 loaded)",
    }),
  ).toBeVisible();
  await expect(
    picker.getByRole("group", {
      name: "Shared service (failed-service) (1 loaded)",
    }),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("picker-desktop.png"),
    animations: "disabled",
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: testInfo.outputPath("picker-mobile.png"),
    animations: "disabled",
  });
  for (const element of [form, picker]) {
    expect(
      await element.evaluate(
        (node) => node.scrollWidth <= node.clientWidth + 1,
      ),
    ).toBe(true);
    const bounds = await element.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(391);
  }
  await page.setViewportSize({ width: 1440, height: 1000 });
  for (let member = 1; member <= 2; member++) {
    await search.fill(`Local pool member ${member}`);
    const option = picker.getByRole("option", {
      name: `Local pool member ${member}`,
      exact: true,
    });
    await option.click();
    await expect(option).toHaveAttribute("aria-selected", "true");
    await expect(picker).toBeVisible();
    if (member === 1) {
      await search.fill("");
      await expect(picker.getByRole("option")).toHaveCount(3);
    }
    if (member === 2) {
      for (let cycle = 0; cycle < 3; cycle++) {
        await option.click();
        await expect(option).toHaveAttribute("aria-selected", "false");
        await option.click();
        await expect(option).toHaveAttribute("aria-selected", "true");
      }
    }
  }
  await picker.getByRole("button", { name: "Done", exact: true }).click();
  await expect(trigger).toBeFocused();
  for (let member = 1; member <= 2; member++) {
    await form
      .getByRole("checkbox", {
        name: `Confirm API compatibility for member ${member}`,
      })
      .check();
  }
  await expect(
    form.getByRole("button", { name: "Create pool", exact: true }),
  ).toBeEnabled();
  for (let cycle = 0; cycle < 3; cycle++) {
    await trigger.click();
    await search.press("Escape");
    await expect(trigger).toBeFocused();
    await expect(form).toBeVisible();
  }
  expect(runtimeErrors).toEqual([]);
  expect(
    await page.evaluate(
      () =>
        (window as Window & { poolLayoutErrors: string[] }).poolLayoutErrors,
    ),
  ).toEqual([]);
});

test("opening routing in a fresh pool dialog does not produce observer errors", async ({
  page,
}) => {
  const runtimeErrors: string[] = [];
  await page.addInitScript(() => {
    const errors: string[] = [];
    Object.assign(window, { poolLayoutErrors: errors });
    window.addEventListener(
      "error",
      (event) => {
        if (event.message) errors.push(event.message);
      },
      true,
    );
  });
  page.on("pageerror", (error) => runtimeErrors.push(error.message));
  page.on("console", (entry) => {
    if (entry.type() === "error") runtimeErrors.push(entry.text());
  });
  await mockConnections(page);
  await page.goto("/keys?tab=pools");

  for (let cycle = 0; cycle < 10; cycle++) {
    await page.getByRole("button", { name: /^Create pool$/i }).click();
    const form = page.getByRole("dialog", {
      name: "Create service pool",
      exact: true,
    });
    await form.getByLabel("Name", { exact: true }).fill(`Routing ${cycle}`);
    await form.getByRole("combobox", { name: "Routing" }).click();
    await page
      .getByRole("option", { name: "Weighted balancing", exact: true })
      .click();
    await form.getByRole("button", { name: "Close" }).click();
    await expect(form).toBeHidden();
  }

  expect(runtimeErrors).toEqual([]);
  expect(
    await page.evaluate(
      () =>
        (window as Window & { poolLayoutErrors: string[] }).poolLayoutErrors,
    ),
  ).toEqual([]);
});

test("keeps a long picker within short landscape and mobile viewports", async ({
  page,
}) => {
  const runtimeErrors: string[] = [];
  await page.addInitScript(() => {
    const errors: string[] = [];
    Object.assign(window, { poolLayoutErrors: errors });
    window.addEventListener(
      "error",
      (event) => {
        if (event.message) errors.push(event.message);
      },
      true,
    );
  });
  page.on("pageerror", (error) => runtimeErrors.push(error.message));
  page.on("console", (entry) => {
    if (entry.type() === "error") runtimeErrors.push(entry.text());
  });
  await mockConnections(page, manyCandidates);
  await page.setViewportSize({ width: 1024, height: 600 });
  await page.goto("/keys?tab=pools");
  await page.getByRole("button", { name: /^Create pool$/i }).click();
  const form = page.getByRole("dialog", {
    name: "Create service pool",
    exact: true,
  });
  await form.getByLabel("Name", { exact: true }).fill("Short viewport pool");
  await form.getByRole("button", { name: "Choose connections" }).click();
  const picker = page.getByRole("dialog", {
    name: "Choose pool connections",
    exact: true,
  });
  await expect(picker.getByRole("option")).toHaveCount(12);

  for (const [width, height] of [
    [1024, 600],
    [780, 390],
    [390, 480],
  ] as const) {
    await page.setViewportSize({ width, height });
    await expect
      .poll(async () => {
        const box = await picker.boundingBox();
        return Boolean(
          box &&
          box.x >= 0 &&
          box.y >= 0 &&
          box.x + box.width <= width + 1 &&
          box.y + box.height <= height + 1,
        );
      })
      .toBe(true);
    expect(
      await picker.evaluate((node) => node.scrollWidth <= node.clientWidth + 1),
    ).toBe(true);
    const firstOption = picker.getByRole("option").first();
    const scrollViewport = picker.getByRole("listbox").locator("..");
    await firstOption.scrollIntoViewIfNeeded();
    await expect
      .poll(async () => {
        const listBox = await scrollViewport.boundingBox();
        const optionBox = await firstOption.boundingBox();
        return Boolean(
          listBox &&
          optionBox &&
          optionBox.y >= listBox.y - 1 &&
          optionBox.y + optionBox.height <= listBox.y + listBox.height + 1,
        );
      })
      .toBe(true);
  }
  expect(runtimeErrors).toEqual([]);
  expect(
    await page.evaluate(
      () =>
        (window as Window & { poolLayoutErrors: string[] }).poolLayoutErrors,
    ),
  ).toEqual([]);
});
