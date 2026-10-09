import { expect, test, type Locator, type Page } from "@playwright/test";

async function mockServices(page: Page) {
  const keys = Array.from({ length: 9 }, (_, service) =>
    Array.from({ length: service === 8 ? 1 : 7 }, (_, connection) => ({
      id: `service-${service}-connection-${connection}`,
      label: `Account ${connection + 1}`,
      slug: `service-${service}-account-${connection}`,
      endpoint_url: "https://example.test/v1",
      endpoint_id: `endpoint-${service}-${connection}`,
      api_key_id: `credential-${service}-${connection}`,
      credential_type: "api_key",
      auth_method: "bearer",
      auth_key_name: "Authorization",
      status: "active",
      catalog_service_id: `catalog-${service}`,
      catalog_service_slug: `service-${service}`,
      catalog_service_name: `Service ${service + 1}`,
      node_id: null,
      node_priority: 0,
      is_active: true,
      auto_connected: false,
      ws_frame_injections: [],
      expires_at: null,
      last_used_at: null,
      error_message: null,
      created_at: "2026-10-01T00:00:00Z",
      service_type: "http",
      credential_source: { type: "personal" },
    })),
  ).flat();
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname.replace("/api/v1", "");
    const responses: Record<string, unknown> = {
      "/users/me": {
        id: "44444444-4444-4444-8444-444444444444",
        email: "card-layout@example.test",
        display_name: "Local review",
        role: "user",
        is_active: true,
        email_verified: true,
        created_at: "2026-10-01T00:00:00Z",
      },
      "/orgs": { orgs: [], organizations: [] },
      "/catalog": { entries: [] },
      "/keys": { keys },
      "/api-keys": { keys: [] },
      "/service-pools": { pools: [] },
      "/service-insights": { connections: [] },
      "/nodes": { nodes: [] },
      "/providers": { providers: [] },
      "/services": { services: [] },
      "/user-services": { services: [] },
      "/runtime-config": {
        release_integrity: {
          enabled: false,
          manifest_url: null,
          verification_ttl_secs: 300,
        },
      },
    };
    await route.fulfill({ json: responses[path] ?? {} });
  });
}

async function geometry(card: Locator) {
  return card.evaluate((node) => {
    const header = node.querySelector<HTMLElement>(".service-card-header")!;
    const surface = header.firstElementChild!;
    const toolbar = document.querySelector(".service-filter-toolbar")!;
    const filters = toolbar.firstElementChild!;
    const body = document.getElementById(
      header
        .querySelector("button[aria-controls]")!
        .getAttribute("aria-controls")!,
    )!;
    const bounds = node.getBoundingClientRect();
    const head = header.getBoundingClientRect();
    const content = body.getBoundingClientRect();
    return {
      openingGap: bounds.top - filters.getBoundingClientRect().bottom,
      headerGap: head.top - filters.getBoundingClientRect().bottom,
      leftEdge: head.left - bounds.left,
      rightEdge: head.right - bounds.right,
      bodyLeft: content.left - head.left,
      bodyRight: head.right - content.right,
      join: content.top - head.bottom,
      borderLeft: getComputedStyle(surface).borderLeftWidth,
      borderRight: getComputedStyle(surface).borderRightWidth,
      borderBottom: getComputedStyle(surface).borderBottomWidth,
    };
  });
}

for (const scenario of [
  {
    name: "desktop light",
    width: 1440,
    height: 1000,
    theme: "light",
    reduced: false,
  },
  {
    name: "desktop dark reduced motion",
    width: 1440,
    height: 1000,
    theme: "dark",
    reduced: true,
  },
  {
    name: "mobile dark",
    width: 390,
    height: 844,
    theme: "dark",
    reduced: false,
  },
  {
    name: "mobile light reduced motion",
    width: 390,
    height: 844,
    theme: "light",
    reduced: true,
  },
] as const) {
  test(`expanded service cards settle below the filters and keep connected borders: ${scenario.name}`, async ({
    page,
  }, testInfo) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (entry) => {
      if (entry.type() === "error") errors.push(entry.text());
    });
    await page.setViewportSize({
      width: scenario.width,
      height: scenario.height,
    });
    await page.emulateMedia({
      reducedMotion: scenario.reduced ? "reduce" : "no-preference",
    });
    await page.addInitScript((mode) => {
      localStorage.setItem(
        "nyxid.theme",
        JSON.stringify({ state: { mode }, version: 1 }),
      );
    }, scenario.theme);
    await mockServices(page);
    await page.goto("/keys?view=routing");
    await expect(page.locator("html")).toHaveClass(
      new RegExp(`theme-${scenario.theme}`),
    );

    for (const name of ["Service 1", "Service 5", "Service 9"]) {
      await page
        .getByRole("button", {
          name: `Expand ${name} connections`,
          exact: true,
        })
        .click();
      const card = page.getByRole("region", { name, exact: true });
      await expect(
        card.getByRole("button", { name: `Collapse ${name} connections` }),
      ).toBeVisible();
      // Include the close/reflow/reveal sequence when switching open cards.
      await page.waitForTimeout(900);
      const opened = await geometry(card);
      expect(Math.abs(opened.openingGap - 8)).toBeLessThan(1);
      expect(Math.abs(opened.join)).toBeLessThan(1);
      expect(opened.leftEdge).toBe(0);
      expect(opened.rightEdge).toBe(0);

      if (name === "Service 9") {
        // A short final card also aligns even though its body fits on screen.
        await page
          .getByRole("button", { name: `Collapse ${name} connections` })
          .click();
        await page.waitForTimeout(400);
        expect(
          await card.locator("..").evaluate((grid) => grid.style.paddingBottom),
        ).toBe("");
        continue;
      }
      await page.locator("main").evaluate((main) => {
        main.scrollTop += 140;
      });
      await expect(card.locator(".service-card-header")).toHaveAttribute(
        "data-stuck",
        "true",
      );
      const pinned = await geometry(card);
      expect(Math.abs(pinned.headerGap - 8)).toBeLessThan(1);
      expect(pinned.leftEdge).toBe(0);
      expect(pinned.rightEdge).toBe(0);
      expect(pinned.bodyLeft).toBe(1);
      expect(pinned.bodyRight).toBe(1);
      expect(pinned.borderLeft).toBe("1px");
      expect(pinned.borderRight).toBe("1px");
      expect(pinned.borderBottom).toBe("1px");
      if (name === "Service 5")
        await page.screenshot({
          path: testInfo.outputPath("connected-card.png"),
        });
    }
    const search = page.getByLabel("Search services and connections");
    await search.fill("Service 9");
    await search.press("Enter");
    await expect(
      page.getByRole("button", { name: "Expand Service 1 connections" }),
    ).toHaveCount(0);
    await page
      .getByRole("button", { name: "Expand Service 9 connections" })
      .click();
    await page.waitForTimeout(900);
    const filtered = await geometry(
      page.getByRole("region", { name: "Service 9", exact: true }),
    );
    expect(Math.abs(filtered.openingGap - 8)).toBeLessThan(1);
    expect(errors).toEqual([]);
  });
}
