import { expect, test, type Page } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import { newView } from "../src/lib/usage-analytics";
import { nyxbotModelFixture } from "../src/test/nyxbot-model-fixture";
import { usageFixture } from "../src/test/admin-usage-fixture";

async function setup(page: Page) {
  await page.clock.setFixedTime(new Date("2026-10-16T12:00:00Z"));
  await mockDashboard(page);
  await page.route("**/api/v1/admin/usage/identities?**", (route) =>
    route.fulfill({ json: { options: [], total: 0 } }),
  );
  await page.route("**/api/v1/admin/usage/workspace", (route) =>
    route.fulfill({
      json: {
        revision: 1,
        config: { version: 1, draft: newView(), saved_views: [] },
      },
    }),
  );
  await page.route("**/api/v1/admin/usage?**", (route) =>
    route.fulfill({ json: usageFixture() }),
  );
  await page.route("**/api/v1/admin/assistant/model-usage?**", (route) => {
    const params = new URL(route.request().url()).searchParams;
    return route.fulfill({
      json: nyxbotModelFixture(
        params.get("from")!,
        params.get("to")!,
        params.has("unknown_model") ? null : (params.get("model") ?? undefined),
      ),
    });
  });
  await page.route("**/api/v1/admin/assistant/analytics?**", (route) => {
    const params = new URL(route.request().url()).searchParams;
    return route.fulfill({
      json: {
        from: params.get("from"),
        to: params.get("to"),
        generated_at: "2026-10-16T12:00:00Z",
        active_sessions: 126,
        active_accounts: 42,
        running_now: 8,
        outcomes: {
          completed: 347,
          failed: 12,
          stopped: 5,
          with_tools: 218,
          with_tool_errors: 19,
        },
        daily_sessions: [12, 18, 16, 24, 21, 29, 32].map((sessions, i) => ({
          day: `2026-10-${10 + i}`,
          sessions,
        })),
        daily_turns: [31, 42, 39, 58, 47, 71, 76].map((turns, i) => ({
          day: `2026-10-${10 + i}`,
          turns,
        })),
      },
    });
  });
  await page.route("**/api/v1/admin/assistant/feedback?**", (route) => {
    const filter = new URL(route.request().url()).searchParams.get("vote");
    const recent = [
      {
        message_id: "reply-negative",
        vote: "down",
        comment:
          "Found the right document, but missed the dates in the final answer.",
        created_at: "2026-10-16T11:22:00Z",
        updated_at: "2026-10-16T11:22:00Z",
        context: {
          agent_kind: "nyxbot",
          model: "nyxagent/research",
          latency_ms: 12900,
          tool_calls: 4,
          tool_errors: 1,
          tool_labels: ["nyx__search_tools", "drive__search"],
        },
      },
      {
        message_id: "reply-positive",
        vote: "up",
        comment:
          "Exactly what I needed. The summary and source links saved me a lot of time.",
        created_at: "2026-10-16T10:09:00Z",
        updated_at: "2026-10-16T10:09:00Z",
        context: {
          agent_kind: "specialist",
          model: "nyxagent/chat",
          latency_ms: 3800,
          tool_calls: 2,
          tool_errors: 0,
        },
      },
      {
        message_id: "reply-legacy",
        vote: "up",
        created_at: "2026-10-15T09:10:00Z",
        updated_at: "2026-10-15T09:10:00Z",
      },
    ].filter((row) => !filter || row.vote === filter);
    return route.fulfill({
      json: {
        counts: [3, 5, 4, 8, 6, 9, 10].map((up, i) => ({
          day: `2026-10-${10 + i}`,
          up,
          down: i % 3,
        })),
        recent,
      },
    });
  });
}

test("NyxBot has its own charts and feedback list, independent of billing navigation", async ({
  page,
}) => {
  await setup(page);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/admin/usage?tab=nyxbot");
  await expect(
    page.getByRole("tab", { name: "NyxBot", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("heading", { name: "Session activity" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Feedback over time" }),
  ).toBeVisible();
  await expect(page.getByText("126", { exact: true })).toBeVisible();
  await expect(page.getByText("88%", { exact: true })).toBeVisible();
  await expect(
    page.getByText("reply-negative", { exact: true }),
  ).not.toBeVisible();
  await page.getByRole("combobox", { name: "Vote", exact: true }).click();
  await page.getByRole("option", { name: "Not helpful", exact: true }).click();
  await expect(
    page.getByText("Exactly what I needed.", { exact: false }),
  ).toHaveCount(0);
  await expect(page.getByText("88%", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Clear filters" }).click();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(
    page.getByText("Exactly what I needed.", { exact: false }),
  ).toBeVisible();
  await page.setViewportSize({ width: 1440, height: 1800 });
  await page.screenshot({
    path: "test-results/nyxbot-usage-desktop.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Switch to light mode" }).click();
  await expect(page.locator("html")).toHaveClass(/theme-light/);
  await page.screenshot({
    path: "test-results/nyxbot-usage-light.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.getByRole("button", { name: "Switch to dark mode" }).click();
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await expect(page).toHaveURL(/tab=list/);
  await expect(
    page.getByRole("heading", { name: "Recent feedback" }),
  ).toHaveCount(0);
  await page.goBack();
  await expect(
    page.getByRole("heading", { name: "Recent feedback" }),
  ).toBeVisible();
  await page.goForward();
  await expect(
    page.getByRole("tab", { name: "List", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
});

test("NyxBot stays usable on mobile and rejects an excessive date range", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await setup(page);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/admin/usage?tab=nyxbot");
  await expect(
    page.getByRole("heading", { name: "Recent feedback" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "test-results/nyxbot-usage-mobile.png",
    fullPage: true,
  });
  await page
    .getByRole("heading", { name: "Recent feedback" })
    .scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "test-results/nyxbot-usage-mobile-feedback.png",
  });
  await page.getByLabel("From", { exact: true }).fill("2026-01-01");
  await expect(page.getByRole("alert")).toContainText("at most 31 days");
  await expect(
    page.getByRole("heading", { name: "Recent feedback" }),
  ).toHaveCount(0);
});

test("feedback remains useful when activity fails, with an honest empty helpful rate", async ({
  page,
}) => {
  await setup(page);
  await page.route("**/api/v1/admin/assistant/analytics?**", (route) =>
    route.fulfill({
      status: 503,
      json: { error: { message: "Unavailable", code: 5000 } },
    }),
  );
  await page.route("**/api/v1/admin/assistant/feedback?**", (route) =>
    route.fulfill({ json: { counts: [], recent: [] } }),
  );
  await page.goto("/admin/usage?tab=nyxbot");
  await expect(page.getByRole("alert")).toContainText(
    "Unable to load NyxBot activity",
  );
  await expect(
    page.getByText("No votes in this period", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("0%", { exact: true })).toHaveCount(0);
  await expect(
    page.getByText("Votes and comments appear here when users rate a reply."),
  ).toBeVisible();
});

test("usage viewers see the full NyxBot tab", async ({
  page,
}) => {
  await setup(page);
  await page.route("**/api/v1/users/me", (route) =>
    route.fulfill({
      json: {
        id: "usage-viewer",
        email: "usage@example.test",
        is_admin: false,
        role: "user",
        is_active: true,
        email_verified: true,
        created_at: "2026-10-01T00:00:00Z",
        capabilities: { read_admin_usage: true },
      },
    }),
  );
  const assistantRequests: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/admin/assistant/"))
      assistantRequests.push(request.url());
  });
  await page.goto("/admin/usage?tab=nyxbot");
  await expect(page).toHaveURL(/tab=nyxbot/);
  for (const name of ["Dashboard", "List"])
    await expect(page.getByRole("tab", { name, exact: true })).toBeVisible();
  await expect(
    page.getByRole("tab", { name: "NyxBot", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  for (const name of [
    "Cost and billing by model",
    "Session activity",
    "Feedback over time",
  ])
    await expect(page.getByRole("heading", { name })).toBeVisible();
  await expect(
    page.getByText("Found the right document", { exact: false }),
  ).toBeVisible();
  for (const endpoint of ["model-usage", "analytics", "feedback"])
    expect(
      assistantRequests.some((url) =>
        url.includes(`/admin/assistant/${endpoint}?`),
      ),
    ).toBe(true);
});

test("model trends, exact credits and keyboard drilldown share the date/model filter", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/admin/usage?tab=nyxbot");
  await expect(
    page.getByRole("heading", { name: "Cost and billing by model" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Gross credits · Daily UTC" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Requests · Daily UTC" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Total tokens · Daily UTC" }),
  ).toBeVisible();
  const model = page
    .getByRole("button", { name: "gpt-5.4", exact: true })
    .first();
  await model.focus();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("status").filter({ hasText: "Model: gpt-5.4" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "claude-sonnet-4-5", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText("0.000000000001", { exact: true }).first(),
  ).toBeVisible();
  await page.getByRole("button", { name: "All models", exact: true }).click();
  await expect(
    page
      .getByRole("button", { name: "claude-sonnet-4-5", exact: true })
      .first(),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Unknown model", exact: true })
    .first()
    .click();
  await expect(
    page.getByRole("status").filter({ hasText: "Unknown model selected" }),
  ).toBeVisible();
});

for (const mobile of [false, true]) {
  test(`model report sections have scrolled ${mobile ? "mobile" : "desktop"} proof`, async ({
    page,
  }) => {
    await page.setViewportSize(
      mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 },
    );
    await setup(page);
    await page.goto("/admin/usage?tab=nyxbot");
    await expect(
      page.getByRole("heading", { name: "Cost and billing by model" }),
    ).toBeVisible();
    for (const [section, target] of [
      ["cost", page.getByText("Gross cost", { exact: true })],
      [
        "traffic",
        page.getByRole("heading", { name: "Traffic by model", exact: true }),
      ],
      [
        "services",
        page.getByRole("heading", { name: "Model services", exact: true }),
      ],
      [
        "accounts",
        page.getByRole("heading", {
          name: "Top billing accounts",
          exact: true,
        }),
      ],
    ] as const) {
      await target.evaluate((element) => {
        const main = element.closest("main");
        if (main)
          main.scrollTop +=
            element.getBoundingClientRect().top -
            main.getBoundingClientRect().top -
            24;
      });
      await expect(target).toBeInViewport();
      if (section === "services") {
        await expect(
          page.getByRole("table", { name: "Model services data" }),
        ).toBeVisible();
      }
      if (section === "accounts") {
        await expect(
          page.getByRole("table", { name: "Top billing accounts data" }),
        ).toBeVisible();
        await page.getByText("Models by billing account").click();
        await expect(
          page.getByRole("heading", { name: "Alice Research" }),
        ).toBeVisible();
        await expect(
          page.getByRole("table", { name: "Alice Research usage by model" }),
        ).toBeVisible();
      }
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= window.innerWidth,
        ),
      ).toBe(true);
      await page.screenshot({
        path: `test-results/nyxbot-model-${mobile ? "mobile" : "desktop"}-${section}.png`,
        animations: "disabled",
      });
    }
    await page
      .getByRole("textbox", { name: "Billing model" })
      .fill("no-such-model");
    await page
      .getByRole("button", { name: "Apply model", exact: true })
      .click();
    await expect(
      page.getByText(/No attributed model usage in this period for this model/),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Top billing accounts", exact: true }),
    ).toHaveCount(0);
  });
}
