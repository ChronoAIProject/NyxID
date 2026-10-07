import { billingWallet, billingUsage, billingGrant, billingAllowance, billingCatalog, billingRow } from "../src/test/billing-fixture";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openAssistant, sendMessage, stopButton } from "./helpers";

async function setTextScale(scope: Page | Locator, scale: number) {
  const slider = scope.getByRole("slider", { name: "Text size", exact: true });
  await expect(slider).toHaveValue("1");
  for (let step = 0; step < Math.round((scale - 1) * 100); step++) {
    await slider.press("ArrowRight");
  }
  await expect(slider).toHaveValue(String(scale));
}

const unexpectedRequests = new WeakMap<Page, string[]>();

test.beforeEach(async ({ page, context }) => {
  const unexpected: string[] = [];
  unexpectedRequests.set(page, unexpected);
  // Keep chat fixtures while account requests use this spec's populated HTTP fixtures.
  await context.route("**/src/lib/mock-data.ts*", async (route) => {
    const response = await route.fetch({ headers: { ...route.request().headers(), "if-none-match": "", "if-modified-since": "" } });
    const source = await response.text();
    const signature = 'export function getMockResponse(endpoint, method = "GET", body) {';
    expect(source).toContain(signature);
    await route.fulfill({ response, body: source.replace(signature, `${signature}
      const fixturePath = endpoint.split("?")[0];
      if (fixturePath.startsWith("/billing/") || ["/catalog", "/public/config", "/sessions"].includes(fixturePath)) return undefined;`) });
  });
  await context.route("**/api/v1/**", (route) => {
    unexpected.push(
      `${route.request().method()} ${new URL(route.request().url()).pathname}`,
    );
    return route.fulfill({
      status: 500,
      json: { message: "Unexpected request outside fixtures" },
    });
  });
  await context.route("**/api/v1/approvals/requests?**", (route) =>
    route.fulfill({ json: { requests: [], total: 0 } }),
  );
  await context.route("**/api/v1/users/me", (route) =>
    route.fulfill({
      json: {
        id: "d4f5a6b7-c8d9-4e0f-a1b2-c3d4e5f60718",
        email: "reader@example.com",
        display_name: "Reader",
        email_verified: true,
        mfa_enabled: false,
        is_admin: false,
        is_active: true,
        created_at: "2026-01-01T00:00:00Z",
        capabilities: {
          billing_available: true,
          enabled_features: ["assistant:nyxagent-engine"],
        },
      },
    }),
  );
  await context.route("**/api/v1/public/config", (route) =>
    route.fulfill({ json: { mcp_url: `https://nyxid.test/${"long-mcp-endpoint/".repeat(18)}mcp` } }),
  );
  await context.route("**/api/v1/sessions", (route) =>
    route.fulfill({ json: [{ id: "session-1", user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/128", ip_address: "203.0.113.1", created_at: "2026-10-01T00:00:00Z", last_active_at: "2026-10-07T00:00:00Z", is_current: false }] }),
  );
  const fixtures: Record<string, unknown> = {
    "/billing/wallet": billingWallet(),
    "/billing/usage": billingUsage([billingRow({ model: "Example model", api_key_name: "Chat agent", api_key_id: "key-1" })]),
    "/billing/grants": { grants: [billingGrant()], page: 1, per_page: 10, total: 1 },
    "/billing/allowances": { allowances: [billingAllowance()] },
    "/billing/topups": { owner_id: "test-user", topups: [{ id: "topup-1", created_at: "2026-10-01T00:00:00Z", amount_credits: 100, status: "paid", receipt_available: true, invoice_number: "INV-1", credits_expire_at: "2027-10-01T00:00:00Z" }], page: 1, per_page: 10, total: 1 },
    "/catalog": { entries: billingCatalog },
  };
  for (const [path, json] of Object.entries(fixtures)) {
    await context.route(`**/api/v1${path}*`, (route) => route.fulfill({ json }));
  }
  await context.route("**/api/v1/assistant/nyxagent/machines?**", (route) => route.fulfill({ json: [] }));
  await context.route("**/api/v1/orgs", (route) => route.fulfill({ json: { orgs: [] } }));
  await context.route("**/api/v1/channel-bots?**", (route) => route.fulfill({ json: { bots: [], total: 0 } }));
});

test.afterEach(({ page }) => {
  expect(unexpectedRequests.get(page)).toEqual([]);
});

for (const width of [320, 390, 768, 1024, 1280]) {
  for (const textScale of [1, 1.25]) {
    test(`all Settings tabs stay usable at ${width}px with ${textScale * 100}% text`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
      if (width < 768) await page.getByRole("button", { name: "Open chats" }).click();
      await page.getByRole("button", { name: "Account menu" }).click();
      await page
        .getByRole("menuitem", { name: "Settings", exact: true })
        .click();
      await expect(
        page.getByRole("heading", { name: "Account Settings" }),
      ).toBeVisible();
      await expect(
        page.getByRole("button", { name: "Close chats" }),
      ).toHaveCount(0);
      await page.getByRole("tab", { name: "Display", exact: true }).click();
      await setTextScale(page, textScale);
      for (const label of [
        "Profile",
        "Security",
        "Sessions",
        "MCP",
        "Display",
        "Privacy",
      ]) {
        const tab = page.getByRole("tab", { name: label, exact: true });
        await tab.click();
        await expect(tab).toHaveAttribute("data-state", "active");
        await expect(page.getByRole("tabpanel")).toBeVisible();
        expect(new URL(page.url()).pathname).toBe("/assistant");
        expect(new URL(page.url()).searchParams.get("panel")).toBe("settings");
        const bounds = await page.getByRole("dialog").evaluate((main) => ({
          width: main.clientWidth,
          scrollWidth: main.scrollWidth,
        }));
        expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.width + 1);
      }
    });
  }
}

test("account menus navigate to NyxBot settings and dismiss same-URL keyboard navigation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Open chats" }).click();
  await page.getByRole("button", { name: "Account menu" }).click();
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Account Settings" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Open chats" }).click();
  await page.getByRole("button", { name: "Account menu" }).click();
  await page
    .getByRole("menuitem", { name: "NyxBot settings", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "NyxBot settings" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Close chats" })).toHaveCount(
    0,
  );
  await expect(page.getByRole("dialog", { name: "NyxBot settings" })).toBeVisible();

  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Open chats" }).click();
  await page.getByRole("button", { name: "Account menu" }).click();
  const nyxbot = page.getByRole("menuitem", {
    name: "NyxBot settings",
    exact: true,
  });
  await nyxbot.focus();
  await nyxbot.press("Enter");
  await expect(page.getByRole("dialog", { name: "NyxBot settings" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Close chats" })).toHaveCount(
    0,
  );

  const headerTrigger = page.getByRole("button", { name: "User menu" });
  await page.keyboard.press("Escape");
  await headerTrigger.click();
  await expect(
    page.getByRole("menuitem", {
      name: "Notification settings (opens in Studio)",
    }),
  ).toBeVisible();
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Account Settings" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Open chats" }).click();
  await page.getByRole("button", { name: "Account menu" }).click();
  const settings = page.getByRole("menuitem", {
    name: "Settings",
    exact: true,
  });
  await settings.focus();
  await settings.press("Enter");
  await expect(page.getByRole("dialog", { name: "Account Settings" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Close chats" })).toHaveCount(
    0,
  );

  await page.keyboard.press("Escape");
  await headerTrigger.click();
  await page
    .getByRole("menuitem", { name: "NyxBot settings", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "NyxBot settings" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await headerTrigger.click();
  await expect(page.getByRole("menu")).toBeVisible();
});

test("keeps the shell visible while account content loads", async ({
  page,
}) => {
  let release: (() => void) | undefined;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/src/pages/settings.tsx", async (route) => {
    await pending;
    await route.continue();
  });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
  await page.getByRole("button", { name: "User menu" }).click();
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "Loading Account Settings..." }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "User menu", includeHidden: true })).toBeAttached();
  await expect(
    page.getByRole("button", { name: "Account menu", includeHidden: true }),
  ).toBeAttached();
  release?.();
  await expect(
    page.getByRole("heading", { name: "Account Settings" }),
  ).toBeVisible();
});

async function expectUsable(control: Locator) {
  await expect(control).toBeEnabled();
  await control.scrollIntoViewIfNeeded();
  await expect(control).toBeVisible();
  const bounds = await control.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    let left = 0;
    let right = window.innerWidth;
    for (
      let parent = element.parentElement;
      parent;
      parent = parent.parentElement
    ) {
      if (
        ["hidden", "clip", "auto", "scroll"].includes(
          getComputedStyle(parent).overflowX,
        )
      ) {
        const box = parent.getBoundingClientRect();
        left = Math.max(left, box.left + parent.clientLeft);
        right = Math.min(
          right,
          box.left + parent.clientLeft + parent.clientWidth,
        );
      }
    }
    return {
      left: rect.left,
      right: rect.right,
      visibleLeft: left,
      visibleRight: right,
    };
  });
  expect(bounds.left).toBeGreaterThanOrEqual(bounds.visibleLeft - 1);
  expect(bounds.right).toBeLessThanOrEqual(bounds.visibleRight + 1);
}

for (const width of [320, 390, 768, 1024, 1280]) {
  for (const textScale of [1, 1.25]) {
    test(`populated NyxBot settings remain usable at ${width}px with ${textScale * 100}% text`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
      await page.getByRole("button", { name: "User menu" }).click();
      await page
        .getByRole("menuitem", { name: "Settings", exact: true })
        .click();
      await page.getByRole("tab", { name: "Display", exact: true }).click();
      await setTextScale(page, textScale);
      await page.keyboard.press("Escape");
      await page
        .getByRole("button", { name: "NyxBot settings", exact: true })
        .click();
      await expect(
        page.getByRole("heading", { name: "NyxBot settings" }),
      ).toBeVisible();
      await expect(page).toHaveTitle("nyxid - Home");
      await expect(
        page.getByRole("button", { name: "NyxBot settings", exact: true, includeHidden: true }),
      ).toHaveAttribute("aria-expanded", "true");
      await expect(page.getByRole("dialog", { name: "NyxBot settings" })).toBeVisible();

      await expectUsable(
        page.getByRole("switch", {
          name: "Confirm destructive actions",
          exact: true,
        }),
      );
      for (const label of [
        "Automatic task continuations",
        "Specialists working at once",
        "Group hand-offs per message",
        "Group hand-offs per hour",
      ]) {
        await expectUsable(
          page.getByRole("spinbutton", { name: label, exact: true }),
        );
      }

      const live = page.getByRole("spinbutton", {
        name: "Live specialists",
        exact: true,
      });
      await expectUsable(live);
      await live.fill("12");
      await expectUsable(
        page.getByRole("button", { name: "Save settings", exact: true }),
      );
      await page
        .getByRole("button", { name: "Save settings", exact: true })
        .click();
      await expect(
        page.getByRole("button", { name: "Save settings", exact: true }),
      ).toBeDisabled();

      await expectUsable(
        page.getByRole("button", { name: "Timezone and automation budgets" }),
      );
      await page
        .getByRole("button", { name: "Timezone and automation budgets" })
        .click();
      const timezone = page.getByRole("combobox", {
        name: "Timezone",
        exact: true,
      });
      await expectUsable(timezone);
      await timezone.click();
      await page
        .getByRole("textbox", { name: "Search timezones" })
        .fill("Asia/Singapore");
      await page
        .getByRole("option", { name: /^Asia\/Singapore(?: \(browser\))?$/ })
        .click();
      for (const label of [
        "Minimum interval (minutes)",
        "Runs per hour",
        "Runs per day",
      ]) {
        await expectUsable(
          page.getByRole("spinbutton", { name: label, exact: true }),
        );
      }
      const hourly = page.getByRole("spinbutton", {
        name: "Runs per hour",
        exact: true,
      });
      await hourly.fill("4");
      await expectUsable(
        page.getByRole("button", { name: "Save preferences", exact: true }),
      );
      await page
        .getByRole("button", { name: "Save preferences", exact: true })
        .click();
      await expect(
        page.getByRole("button", { name: "Save preferences", exact: true }),
      ).toHaveCount(0);
      await page
        .getByRole("button", { name: "Timezone and automation budgets" })
        .click();
      await expect(hourly).toHaveValue("4");

      const available = page.getByRole("list", {
        name: "Channel bots you can connect",
      });
      const selector = available.getByRole("combobox", {
        name: "Agent for NyxID Approvals",
        exact: true,
      });
      await expectUsable(selector);
      await selector.click();
      await page
        .getByRole("option", { name: "researcher", exact: true })
        .click();
      const connect = available.getByRole("button", {
        name: "Connect NyxID Approvals",
        exact: true,
      });
      await expectUsable(connect);
      await connect.click();
      const ownerLink = page.getByRole("region", {
        name: "Owner link",
        exact: true,
      });
      await expect(ownerLink).toContainText("nyxlink_");
      await expectUsable(
        ownerLink.getByRole("link", { name: "Open link", exact: true }),
      );
      await expectUsable(
        ownerLink.getByRole("button", { name: "Copy Link code", exact: true }),
      );
      const code = await ownerLink.locator("code").textContent();
      const renew = ownerLink.getByRole("button", {
        name: "New link",
        exact: true,
      });
      await expectUsable(renew);
      await renew.click();
      await expect(ownerLink.locator("code")).not.toHaveText(code ?? "");

      const connected = page.getByRole("list", {
        name: "Connected channel bots",
        exact: true,
      });
      const assigned = connected.getByRole("combobox", {
        name: "Agent for NyxID Approvals",
        exact: true,
      });
      await expectUsable(assigned);
      await expect(assigned).toHaveText(/researcher/);
      await assigned.click();
      await page.getByRole("option", { name: "NyxBot", exact: true }).click();
      await expect(assigned).toHaveText(/NyxBot/);
      await expectUsable(
        connected.getByRole("button", {
          name: "Chats and who can talk",
          exact: true,
        }),
      );
      await connected
        .getByRole("button", { name: "Chats and who can talk", exact: true })
        .click();
      const chats = connected.getByRole("list", {
        name: "Chats of NyxID Approvals",
        exact: true,
      });
      await expect(chats).toBeVisible();
      const replies = chats.getByRole("combobox", {
        name: "Replies in Team chat",
        exact: true,
      });
      await expectUsable(replies);
      await replies.click();
      await page
        .getByRole("option", { name: "Every message", exact: true })
        .click();
      await expect(replies).toHaveText(/Every message/);
      const members = chats.getByRole("combobox", {
        name: "Who can talk in Team chat",
        exact: true,
      });
      await expectUsable(members);
      await members.click();
      await page.getByRole("option", { name: "Only you", exact: true }).click();
      await expect(members).toHaveText(/Only you/);
      const posts = chats.getByRole("switch", {
        name: "Let the agent post in Team chat on its own",
        exact: true,
      });
      await expectUsable(posts);
      await posts.click();
      await expect(posts).toHaveAttribute("aria-checked", "true");
      await expectUsable(
        chats.getByRole("combobox", {
          name: "Agent for Team chat",
          exact: true,
        }),
      );
      await expectUsable(
        connected.getByRole("combobox", {
          name: "Who can talk to NyxID Approvals in private chats",
          exact: true,
        }),
      );
      const bounds = await page.getByRole("dialog").evaluate((main) => ({
        width: main.clientWidth,
        scrollWidth: main.scrollWidth,
      }));
      expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.width + 1);
      expect(new URL(page.url()).pathname).toBe("/assistant");
      expect(new URL(page.url()).searchParams.get("panel")).toBe("nyxbot");
    });
  }
}

for (const width of [390, 1280]) {
  test(`both account menus and the chat gear reach the page at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
    const gear = page.getByRole("button", {
      name: "NyxBot settings",
      exact: true,
    });
    await expect(gear).toHaveAttribute("aria-expanded", "false");
    await gear.click();
    await expect(
      page.getByRole("heading", { name: "NyxBot settings" }),
    ).toBeVisible();
    for (const label of ["Account menu", "User menu"]) {
      await page.keyboard.press("Escape");
      if (label === "Account menu" && width < 768)
        await page.getByRole("button", { name: "Open chats" }).click();
      await page.getByRole("button", { name: label, exact: true }).click();
      await page
        .getByRole("menuitem", { name: "Settings", exact: true })
        .click();
      await expect(
        page.getByRole("heading", { name: "Account Settings" }),
      ).toBeVisible();
      await page.keyboard.press("Escape");
      if (label === "Account menu" && width < 768)
        await page.getByRole("button", { name: "Open chats" }).click();
      await page.getByRole("button", { name: label, exact: true }).click();
      await page
        .getByRole("menuitem", { name: "NyxBot settings", exact: true })
        .click();
      await expect(
        page.getByRole("heading", { name: "NyxBot settings" }),
      ).toBeVisible();
      await expect(
        page.getByRole("button", { name: "Close chats" }),
      ).toHaveCount(0);
    }
  });
}

test("direct entry and reload initialize the NyxBot page fixtures without a latched mock query", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.__nyxidAssistantHttpFaults = { nyxagentEnabled: true };
  });
  await page.route("**/api/v1/orgs", (route) =>
    route.fulfill({ json: { orgs: [] } }),
  );
  await page.route("**/api/v1/channel-bots?**", (route) =>
    route.fulfill({ json: { bots: [], total: 0 } }),
  );
  await page.goto("/assistant?panel=nyxbot");
  await expect(
    page.getByRole("heading", { name: "NyxBot settings" }),
  ).toBeVisible();
  await expect(
    page.getByRole("spinbutton", { name: "Live specialists", exact: true }),
  ).toHaveValue("8");
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "NyxBot settings" }),
  ).toBeVisible();
  await expect(
    page.getByRole("spinbutton", { name: "Live specialists", exact: true }),
  ).toHaveValue("8");
  expect(new URL(page.url()).searchParams.get("panel")).toBe("nyxbot");
});

test("keeps the shell visible while lazy NyxBot page content loads", async ({
  page,
}) => {
  let release: (() => void) | undefined;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(
    "**/src/components/assistant/nyxbot-settings-content.tsx",
    async (route) => {
      await pending;
      await route.continue();
    },
  );
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
  await page.getByRole("button", { name: "User menu" }).click();
  await page
    .getByRole("menuitem", { name: "NyxBot settings", exact: true })
    .click();
  await expect(
    page.getByRole("status").filter({ hasText: "Loading NyxBot settings..." }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "User menu", includeHidden: true })).toBeAttached();
  await expect(
    page.getByRole("button", { name: "Account menu", includeHidden: true }),
  ).toBeAttached();
  release?.();
  await expect(
    page.getByRole("heading", { name: "NyxBot settings" }),
  ).toBeVisible();
});

test("the group chat header gear opens NyxBot settings", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "New group", exact: true })
    .click();
  const dialog = page.getByRole("dialog", { name: "New group", exact: true });
  await dialog
    .getByRole("textbox", { name: "Name", exact: true })
    .fill("Settings review");
  await dialog
    .getByRole("button", { name: "Create group", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Settings review", exact: true }),
  ).toBeVisible();
  const gear = page.getByRole("button", { name: "NyxBot settings", exact: true });
  await expect(gear).toHaveAttribute("aria-expanded", "false");
  await gear.click();
  await expect(
    page.getByRole("heading", { name: "NyxBot settings", exact: true }),
  ).toBeVisible();
  expect(new URL(page.url()).pathname).toBe("/assistant");
  expect(new URL(page.url()).searchParams.get("panel")).toBe("nyxbot");
});

for (const width of [320, 390, 768, 1024, 1280]) {
  for (const textScale of [1, 1.25]) {
    test(`populated Billing and Usage stay usable at ${width}px with ${textScale * 100}% text`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
      await page.getByRole("button", { name: "User menu" }).click();
      await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
      const settings = page.getByRole("dialog", { name: "Account Settings" });
      await settings.getByRole("tab", { name: "Display" }).click();
      await setTextScale(settings, textScale);
      await page.keyboard.press("Escape");
      const trigger = width < 768 ? "User menu" : "Account menu";
      await page.getByRole("button", { name: trigger }).click();
      await page.getByRole("menuitem", { name: "Billing", exact: true }).click();
      const billing = page.getByRole("dialog", { name: "Billing & Usage" });
      await expect(billing).toBeVisible();
      const addCredits = billing.getByRole("button", { name: "Add credits", exact: true });
      await expectUsable(addCredits);
      await addCredits.click();
      const topup = page.getByRole("dialog", { name: "Add credits", exact: true });
      await expect(topup).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(topup).toHaveCount(0);
      await billing.getByRole("tab", { name: "Usage" }).click();
      await expect(billing.getByText("Usage overview", { exact: true })).toBeVisible();
      await billing.getByText("Explore by service, model or agent").click();
      await billing.locator("summary").filter({ hasText: "Example LLM" }).first().click();
      await billing.getByRole("combobox", { name: "Group by", exact: true }).click();
      await page.getByRole("option", { name: "Model", exact: true }).click();
      await expect(billing.getByText("Example model", { exact: true })).toBeVisible();
      await expectUsable(billing.getByRole("combobox", { name: "Time range", exact: true }));
      const bounds = await billing.evaluate((element) => ({ width: element.clientWidth, scroll: element.scrollWidth }));
      expect(bounds.scroll).toBeLessThanOrEqual(bounds.width + 1);
      await page.keyboard.press("Escape");
      await expect(billing).toHaveCount(0);
      await expect(page.getByRole("button", { name: trigger })).toBeFocused();
      await expect.poll(() => page.evaluate(() => document.body.style.pointerEvents)).not.toBe("none");
      await expect.poll(() => page.evaluate(() => document.body.hasAttribute("data-scroll-locked"))).toBe(false);
    });
  }
}

test("Settings deep link reload and Back preserve the current chat", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
  await page.getByRole("button", { name: "User menu" }).click();
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
  const panel = page.getByRole("dialog", { name: "Account Settings" });
  await panel.getByRole("tab", { name: "Security" }).click();
  const address = page.url();
  await page.reload();
  await expect(panel.getByRole("tab", { name: "Security" })).toHaveAttribute("data-state", "active");
  expect(page.url()).toBe(address);
  await page.goBack();
  await expect(panel).toHaveCount(0);
  expect(new URL(page.url()).searchParams.has("panel")).toBe(false);
  await page.goForward();
  await expect(panel).toBeVisible();
  await page.keyboard.press("Escape");
  await page.goBack();
  await expect(panel).toBeVisible();
});

test("a panel survives unresolved draft adoption and the completed chat stays mounted", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
  await page.evaluate(() => {
    const original = globalThis.__nyxidAssistantHttpMock;
    if (!original) throw new Error("Missing chat fixture");
    let release: () => void;
    const pending = new Promise<void>((resolve) => { release = resolve; });
    Object.assign(window, { releaseDraft: () => release() });
    globalThis.__nyxidAssistantHttpMock = async (request) => {
      if (request.endpoint === "/assistant/nyxagent/turns" && request.init.method === "POST") await pending;
      return original(request);
    };
  });
  await sendMessage(page, "A delayed draft");
  await page.getByRole("button", { name: "User menu" }).click();
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
  const panel = page.getByRole("dialog", { name: "Account Settings" });
  await expect(panel).toBeVisible();
  await page.evaluate(() => (window as Window & { releaseDraft: () => void }).releaseDraft());
  await expect.poll(() => new URL(page.url()).searchParams.get("c")).toMatch(/^nyxa-/);
  expect(new URL(page.url()).searchParams.get("panel")).toBe("settings");
  await expect(panel).toBeVisible();
  await expect(page.getByRole("main", { includeHidden: true }).getByText("A delayed draft", { exact: true })).toBeAttached();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("main", { includeHidden: true }).getByText("A delayed draft", { exact: true })).toBeVisible();
  await expect(stopButton(page)).not.toBeVisible();
  await expect(page.locator("textarea")).toBeEnabled();
});

test("an agent deep link keeps its panel through the latest-thread redirect", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
  const agent = await page.evaluate(async () => {
    const response = await globalThis.__nyxidAssistantHttpMock?.({ endpoint: "/assistant/nyxagent/agents", init: {} });
    const data = await response?.json() as { agents: { id: string; name: string }[] };
    return data.agents.find((row) => row.name === "researcher")?.id;
  });
  expect(agent).toBeTruthy();
  await page.goto(`/assistant?mock=1&agent=${agent}&panel=settings&panelTab=security`);
  const panel = page.getByRole("dialog", { name: "Account Settings" });
  await expect(panel.getByRole("tab", { name: "Security" })).toHaveAttribute("data-state", "active");
  await expect.poll(() => new URL(page.url()).searchParams.get("c")).toMatch(/^nyxa-/);
  expect(new URL(page.url()).searchParams.get("panel")).toBe("settings");
  await page.keyboard.press("Escape");
  await expect(page.getByText("Found 3 urgent issues: #12, #15 and #18.")).toBeVisible();
});

test("rendered assistant settings links open over their current chat and keep new-tab clicks", async ({ page, context }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await expect(page.getByRole("button", { name: "Account menu", includeHidden: true })).toBeAttached();
  await page.getByRole("button", { name: /^researcher, specialist/ }).click();
  await expect.poll(() => new URL(page.url()).searchParams.get("c")).toMatch(/^nyxa-/);
  const conversation = new URL(page.url()).searchParams.get("c");
  await page.evaluate((id) => {
    const key = "nyxagent-http-fixture-v4";
    const state = JSON.parse(sessionStorage.getItem(key) ?? "null") as { rows: [string, { history: { messages: { role: string; text: string }[] } }][] };
    const row = state.rows.find(([rowId]) => rowId === id);
    const message = row?.[1].history.messages.find((item) => item.role === "assistant");
    if (!message) throw new Error("Missing assistant reply fixture");
    message.text = "Open [Security settings](/assistant?panel=settings&panelTab=security).";
    sessionStorage.setItem(key, JSON.stringify(state));
  }, conversation);
  await page.reload();
  const link = page.getByRole("link", { name: "Security settings", exact: true });
  await expect(link).toBeVisible();
  const popup = context.waitForEvent("page");
  await link.click({ button: "middle" });
  const newTab = await popup;
  await expect(newTab).toHaveURL(/panel=settings/);
  await newTab.close();
  expect(new URL(page.url()).searchParams.get("panel")).toBeNull();
  await link.click();
  await expect(page.getByRole("dialog", { name: "Account Settings" }).getByRole("tab", { name: "Security" })).toHaveAttribute("data-state", "active");
  expect(new URL(page.url()).searchParams.get("c")).toBe(conversation);
  await page.keyboard.press("Escape");
  await expect(link).toBeFocused();
});

test("closing pending top-up then Back never replays it after delayed wallet arrival", async ({ page }) => {
  let release: (() => void) | undefined;
  const pending = new Promise<void>((resolve) => { release = resolve; });
  await page.route("**/api/v1/billing/wallet", async (route) => {
    await pending;
    await route.fulfill({ json: billingWallet() });
  });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page.goto("/assistant?mock=1&panel=billing&panelTab=billing&panelAction=topup");
  const panel = page.getByRole("dialog", { name: "Billing & Usage" });
  await expect(panel).toBeVisible();
  await panel.getByRole("button", { name: "Close", exact: true }).click();
  await expect(panel).toHaveCount(0);
  release?.();
  await page.goBack();
  await expect(panel).toBeVisible();
  expect(new URL(page.url()).searchParams.has("panelAction")).toBe(false);
  await expect(panel.getByRole("button", { name: "Add credits", exact: true })).toBeEnabled();
  await expect(page.getByRole("dialog", { name: "Add credits", exact: true })).toHaveCount(0);
});
