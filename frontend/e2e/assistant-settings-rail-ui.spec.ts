import { mkdir } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  composerInput,
  openAccountMenu,
  openAssistant,
  sendMessage,
} from "./helpers";

test.setTimeout(90_000);

const screenshots = join(
  homedir(),
  ".heca/plans/assistant-settings-rail-ui/after",
);
async function capture(page: Page, name: string) {
  await mkdir(screenshots, { recursive: true });
  await page.screenshot({
    path: join(screenshots, `${name}.png`),
    animations: "disabled",
  });
}
async function theme(page: Page, mode: "light" | "dark") {
  await page.addInitScript((mode) => {
    if (!localStorage.getItem("nyxid.theme"))
      localStorage.setItem(
        "nyxid.theme",
        JSON.stringify({
          version: 2,
          state: { mode, textScale: 1, density: 1 },
        }),
      );
  }, mode);
}
async function settings(page: Page) {
  await openAccountMenu(page);
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
  const panel = page.getByRole("dialog", {
    name: "Account Settings",
    exact: true,
  });
  await expect(panel).toBeVisible();
  return panel;
}
async function settingsApi(page: Page) {
  const writes: { method: string; path: string; body: unknown }[] = [];
  let user = {
    id: "d4f5a6b7-c8d9-4e0f-a1b2-c3d4e5f60718",
    display_name: "Reader",
    email: "reader@example.com",
    email_verified: true,
    has_password: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: "2026-01-01T00:00:00Z",
    capabilities: { enabled_features: ["assistant:nyxagent-engine"] },
    profile_config: {
      security: { skip_mfa_for_approved_sign_ins: true },
      onboarding: { ai_services_completed_at: "2026-01-01T00:00:00Z" },
    },
  };
  await page.route("**/src/lib/mock-data.ts*", async (route) => {
    const response = await route.fetch();
    const source = await response.text();
    const signature =
      'export function getMockResponse(endpoint, method = "GET", body) {';
    expect(source).toContain(signature);
    await route.fulfill({
      response,
      body: source.replace(
        signature,
        `${signature}
      if (["/users/me", "/auth/password/change", "/sessions"].includes(endpoint.split("?")[0])) return undefined;`,
      ),
    });
  });
  await page.route("**/api/v1/users/me", async (route) => {
    if (route.request().method() === "PUT") {
      const body = route.request().postDataJSON();
      writes.push({ method: "PUT", path: "/users/me", body });
      user = { ...user, ...body };
    }
    await route.fulfill({ json: user });
  });
  await page.route("**/api/v1/auth/password/change", async (route) => {
    writes.push({
      method: route.request().method(),
      path: "/auth/password/change",
      body: route.request().postDataJSON(),
    });
    await route.fulfill({ json: {} });
  });
  await page.route("**/api/v1/sessions", (route) =>
    route.fulfill({
      json: [
        {
          id: "current",
          is_current: true,
          user_agent:
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/128",
          ip_address: "203.0.113.1",
          created_at: "2026-11-01T00:00:00Z",
          expires_at: "2026-12-01T00:00:00Z",
        },
        {
          id: "phone",
          is_current: false,
          user_agent:
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) Version/18.0 Mobile Safari/604.1",
          ip_address: "203.0.113.2",
          created_at: "2026-11-02T00:00:00Z",
          expires_at: "2026-12-02T00:00:00Z",
        },
      ],
    }),
  );
  return writes;
}

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  Object.assign(page, { settingsRailErrors: errors });
  await page.route("**/api/v1/assistant/nyxagent/machines?**", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/assistant/nyxagent/cma-sessions?**", (route) =>
    route.fulfill({ json: { sessions: [] } }),
  );
  await page.route("**/api/v1/onboarding/bindings", (route) =>
    route.fulfill({ json: { bindings: [] } }),
  );
});
test.afterEach(async ({ page }, testInfo) => {
  if (testInfo.status !== testInfo.expectedStatus)
    await page.screenshot({
      path: testInfo.outputPath("settings-rail-failure.png"),
    });
  expect(
    (page as Page & { settingsRailErrors: string[] }).settingsRailErrors,
  ).toEqual([]);
});

for (const mode of ["light", "dark"] as const) {
  for (const width of [390, 1600]) {
    test(`Studio shares grouped account forms and preserves tab drafts in ${mode} at ${width}px`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await theme(page, mode);
      const writes = await settingsApi(page);
      await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await page.goto("/settings?mock=1&tab=profile");
      const content = page.getByRole("region", {
        name: "Account settings",
        exact: true,
      });
      await expect(
        content.getByRole("textbox", { name: "Name", exact: true }),
      ).toHaveValue("Reader");
      await expect(
        content.getByRole("region", { name: "Unsaved changes" }),
      ).toHaveCount(0);
      await content
        .getByRole("textbox", { name: "Name", exact: true })
        .fill("Studio Reader");
      await content.getByRole("tab", { name: "Security", exact: true }).click();
      await expect(page).toHaveURL(/tab=security/);
      await expect(
        page.getByRole("dialog", { name: "Discard unsaved changes?" }),
      ).toHaveCount(0);
      await content.getByRole("tab", { name: "Profile", exact: true }).click();
      await expect(
        content.getByRole("textbox", { name: "Name", exact: true }),
      ).toHaveValue("Studio Reader");
      await expect(
        content.locator(
          "[data-grouped-list] [data-grouped-list], [data-slot=card]",
        ),
      ).toHaveCount(0);
      await capture(page, `settings-${mode}-${width}-studio-dirty`);
      await content
        .getByRole("region", { name: "Unsaved changes" })
        .getByRole("button", { name: "Save changes", exact: true })
        .click();
      await page
        .getByRole("dialog", { name: "Review changes" })
        .getByRole("button", { name: "Save changes", exact: true })
        .click();
      await expect(page.getByRole("dialog", { name: "Review changes" })).toHaveCount(0);
      await expect(
        content.getByRole("region", { name: "Unsaved changes" }),
      ).toHaveCount(0);
      expect(writes).toEqual([
        {
          method: "PUT",
          path: "/users/me",
          body: { display_name: "Studio Reader" },
        },
      ]);
      await capture(page, `settings-${mode}-${width}-studio`);
    });

    test(`grouped account settings, reviewed saves and dismissal in ${mode} at ${width}px`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await theme(page, mode);
      const writes = await settingsApi(page);
      await openAssistant(page, { faults: { nyxagentEnabled: true } });
      const panel = await settings(page);
      const surface = await panel.evaluate(
        (element) => getComputedStyle(element).backgroundColor,
      );
      expect(
        await panel
          .locator("[data-compact-chrome]")
          .first()
          .evaluate((element) => getComputedStyle(element).backgroundColor),
      ).toBe(surface);
      await expect(
        panel.getByRole("region", { name: "Unsaved changes" }),
      ).toHaveCount(0);
      for (const label of [
        "Profile",
        "Security",
        "Sessions",
        "MCP",
        "Display",
        "Privacy",
      ]) {
        await panel.getByRole("tab", { name: label, exact: true }).click();
        const tab = panel.getByRole("tabpanel");
        await expect(tab.locator("[data-grouped-list]").first()).toBeVisible();
        await expect(
          tab.locator(
            "[data-grouped-list] [data-grouped-list], [data-slot=card]",
          ),
        ).toHaveCount(0);
        expect(
          await panel.evaluate(
            (element) => element.scrollWidth - element.clientWidth,
          ),
        ).toBeLessThanOrEqual(1);
        await capture(page, `settings-${mode}-${width}-${label.toLowerCase()}`);
      }
      await panel.getByRole("tab", { name: "Profile", exact: true }).click();
      await panel
        .getByRole("textbox", { name: "Name", exact: true })
        .fill("Updated Reader");
      await panel.getByRole("tab", { name: "Security", exact: true }).click();
      await expect(
        page.getByRole("dialog", { name: "Discard unsaved changes?" }),
      ).toHaveCount(0);
      await panel
        .getByLabel("Current Password", { exact: true })
        .fill("old-password");
      await panel
        .getByLabel("New Password", { exact: true })
        .fill("new-password");
      await panel
        .getByLabel("Confirm New Password", { exact: true })
        .fill("new-password");
      await panel.getByRole("tab", { name: "Display", exact: true }).click();
      const bar = panel.getByRole("region", { name: "Unsaved changes" });
      await expect(bar).toBeVisible();
      expect(
        await bar.evaluate(
          (element) => element.closest("[data-form-viewport]") === null,
        ),
      ).toBe(true);
      const barBounds = (await bar.boundingBox())!;
      expect(barBounds.y + barBounds.height).toBeLessThanOrEqual(900);
      await capture(page, `settings-${mode}-${width}-dirty-bar`);
      await bar
        .getByRole("button", { name: "Save changes", exact: true })
        .click();
      const review = page.getByRole("dialog", { name: "Review changes" });
      await expect(review).toContainText("Name: Reader → Updated Reader");
      await expect(review).toContainText(
        "Password: Current password → Changed",
      );
      await expect(review).not.toContainText("old-password");
      await expect(review).not.toContainText("new-password");
      await capture(page, `settings-${mode}-${width}-review`);
      await review
        .getByRole("button", { name: "Save changes", exact: true })
        .click();
      await expect(review).toHaveCount(0);
      await expect(bar).toHaveCount(0);
      expect(writes).toEqual([
        {
          method: "PUT",
          path: "/users/me",
          body: { display_name: "Updated Reader" },
        },
        {
          method: "POST",
          path: "/auth/password/change",
          body: {
            current_password: "old-password",
            new_password: "new-password",
          },
        },
      ]);
      await panel.getByRole("tab", { name: "Profile", exact: true }).click();
      await expect(
        panel.getByRole("textbox", { name: "Name", exact: true }),
      ).toHaveValue("Updated Reader");
      await panel
        .getByRole("textbox", { name: "Name", exact: true })
        .fill("Unsaved draft");
      await page.keyboard.press("Escape");
      const discard = page.getByRole("dialog", {
        name: "Discard unsaved changes?",
      });
      await expect(discard).toBeVisible();
      await discard.getByRole("button", { name: "Keep editing" }).click();
      await expect(
        panel.getByRole("textbox", { name: "Name", exact: true }),
      ).toHaveValue("Unsaved draft");
      await panel.getByRole("button", { name: "Close", exact: true }).click();
      await discard
        .getByRole("button", { name: "Discard", exact: true })
        .click();
      await expect(panel).toHaveCount(0);
      await expect(
        page.getByRole("button", {
          name: width < 768 ? "Open chats" : "Account menu",
          exact: true,
        }),
      ).toBeFocused();
      expect(new URL(page.url()).searchParams.has("panel")).toBe(false);
    });
  }
}

async function seedHistory(page: Page, count = 240) {
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
  await page.evaluate((count) => {
    const key = "nyxagent-http-fixture-v4";
    const fixture = JSON.parse(sessionStorage.getItem(key)!);
    const id = new URLSearchParams(location.search).get("c");
    const row = fixture.rows.find(([key]: [string]) => key === id)[1];
    row.history.messages = Array.from({ length: count }, (_, index) => ({
      id: `rail-message-${index}`,
      seq: index + 1,
      turn_id: `rail-turn-${Math.floor(index / 2)}`,
      role: index % 2 ? "assistant" : "user",
      text: `Review item ${index + 1}: ${index % 2 ? "The findings are ready to check." : "Please check the next item."}`,
      status: "completed",
      error_code: null,
      created_at: new Date(1_790_000_000_000 + index * 1000).toISOString(),
      activities: [],
      attachments: [],
    }));
    row.history.conversation.message_count = count;
    row.history.acknowledgements = [];
    sessionStorage.setItem(key, JSON.stringify(fixture));
  }, count);
  await page.reload({ waitUntil: "domcontentloaded" });
}
async function railBounds(page: Page, rail: Locator) {
  const transcript = (await page
    .locator("[data-chat-transcript]")
    .boundingBox())!;
  const dock = (await page.locator("[data-composer-dock]").boundingBox())!;
  const bounds = (await rail.boundingBox())!;
  const controls = (await page
    .locator("[data-chat-header-controls]")
    .boundingBox())!;
  expect(bounds.x + bounds.width).toBeCloseTo(
    transcript.x + transcript.width,
    0,
  );
  expect(bounds.y).toBeCloseTo(
    Math.max(transcript.y, controls.y + controls.height + 8),
    0,
  );
  expect(bounds.y + bounds.height).toBeCloseTo(
    Math.min(transcript.y + transcript.height, dock.y),
    0,
  );
  expect(bounds.height).toBeGreaterThan(300);
  return bounds;
}
for (const mode of ["light", "dark"] as const) {
  test(`conversation rail stays beside the chat through aside/sidebar resizing in ${mode}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1600, height: 1000 });
    await theme(page, mode);
    await page.route("**/api/v1/assistant/nyxagent/machines?**", (route) =>
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
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    await seedHistory(page);
    const rail = page.getByRole("navigation", { name: "Conversation index" });
    await expect(rail).toBeVisible();
    const preview = page.getByRole("complementary", { name: "Preview" });
    await expect(preview).toBeVisible();
    const divider = page.getByRole("separator", {
      name: "Resize preview column",
    });
    for (const key of ["Home", "End", "ArrowLeft"]) {
      await divider.press(key);
      await expect(async () => {
        await railBounds(page, rail);
      }).toPass();
      const railBox = (await rail.boundingBox())!;
      expect(railBox.x + railBox.width).toBeLessThanOrEqual(
        (await preview.boundingBox())!.x + 1,
      );
    }
    await divider.dblclick();
    await expect(async () => {
      await railBounds(page, rail);
    }).toPass();
    await capture(page, `rail-${mode}-desktop-aside`);
    await expect(rail).toHaveAttribute("data-scroll-edges", "top");
    const initialScroll = await page
      .locator("[data-chat-transcript]")
      .evaluate((element) => element.scrollTop);
    const box = (await rail.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.wheel(0, -200);
    await expect(rail).toHaveAttribute("data-scroll-edges", "top bottom");
    expect(
      await page
        .locator("[data-chat-transcript]")
        .evaluate((element) => element.scrollTop),
    ).toBe(initialScroll);
    expect(await rail.locator("button").count()).toBeLessThan(160);
    const tick = rail.locator('[data-chat-index-target][tabindex="0"]');
    await tick.focus();
    await page.keyboard.press("Home");
    await expect(rail).toHaveAttribute("data-scroll-edges", "bottom");
    const userTick = rail.locator('[data-message-id="rail-message-0"] span');
    const assistantTick = rail.locator(
      '[data-message-id="rail-message-1"] span',
    );
    await expect(userTick).toHaveClass(/text-nyx-secondary-400/);
    expect((await userTick.boundingBox())!.width).toBeCloseTo(8, 1);
    expect((await assistantTick.boundingBox())!.width).toBeCloseTo(16, 1);
    const slots = rail.locator("button");
    expect(
      (await slots.nth(1).boundingBox())!.y -
        (await slots.nth(0).boundingBox())!.y,
    ).toBeCloseTo(7, 1);
    await page.keyboard.press("ArrowDown");
    await expect(assistantTick).toHaveClass(/text-nyx-secondary-400/);
    expect((await userTick.boundingBox())!.width).toBeCloseTo(8, 1);
    const from = (await rail
      .locator('[data-chat-index-target="2"]')
      .boundingBox())!;
    const to = (await rail
      .locator('[data-chat-index-target="8"]')
      .boundingBox())!;
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, {
      steps: 6,
    });
    await page.mouse.up();
    await expect(page.locator("#message-rail-message-8")).toBeInViewport();
    await rail.locator('[aria-current="location"]').focus();
    await page.keyboard.press("End");
    await expect(rail).toHaveAttribute("data-scroll-edges", "top");
    const height = (await rail.boundingBox())!.height;
    await composerInput(page).fill("Draft line\n".repeat(40));
    await page.getByRole("separator", { name: "Resize composer" }).press("End");
    await expect
      .poll(async () => (await rail.boundingBox())!.height)
      .toBeLessThan(height - 200);
    await composerInput(page).fill("");
    await page
      .getByRole("button", { name: "Hide machine", exact: true })
      .click();
    await expect(preview).toHaveCount(0);
    await expect(async () => {
      await railBounds(page, rail);
    }).toPass();
    await capture(page, `rail-${mode}-desktop-no-aside`);
    await page
      .getByRole("button", { name: "Hide sidebar", exact: true })
      .click();
    await expect(async () => {
      await railBounds(page, rail);
    }).toPass();
    await capture(page, `rail-${mode}-desktop-collapsed-sidebar`);
    await page
      .getByRole("button", { name: "Show sidebar", exact: true })
      .click();
    await expect(async () => {
      await railBounds(page, rail);
    }).toPass();
    await page.setViewportSize({ width: 800, height: 1000 });
    await expect(async () => {
      await railBounds(page, rail);
    }).toPass();
    await capture(page, `rail-${mode}-narrow`);
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(rail).toBeHidden();
    expect(
      await page
        .locator("[data-chat-content-layout]")
        .evaluate((element) =>
          getComputedStyle(element)
            .getPropertyValue("--chat-index-width")
            .trim(),
        ),
    ).toBe("");
    expect(
      await page
        .getByRole("main")
        .evaluate((element) => element.scrollWidth - element.clientWidth),
    ).toBeLessThanOrEqual(1);
    await capture(page, `rail-${mode}-mobile`);
  });
}
