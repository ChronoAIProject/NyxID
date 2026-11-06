import { expect, test, type Page } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import { newView } from "../src/lib/usage-analytics";
import { nyxbotModelFixture } from "../src/test/nyxbot-model-fixture";
import { usageFixture } from "../src/test/admin-usage-fixture";

async function setup(page: Page, preview = true) {
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
  await page.route("**/api/v1/admin/usage/free-grants?**", (route) =>
    route.fulfill({
      json: {
        window: { from: "2026-10-10T00:00:00Z", to: "2026-10-17T00:00:00Z" },
        generated_at: "2026-10-16T12:00:00Z",
        issued: "0",
        used: "0",
        interval: "day",
        trend: [],
        total_services: 0,
        service_page: 1,
        services: [],
        grants: 0,
        total_accounts: 0,
        accounts: [],
        page: 1,
        per_page: 50,
      },
    }),
  );
  await page.route("**/api/v1/admin/assistant/feedback/*/thread**", (route) => {
    const previous = new URL(route.request().url()).searchParams.has("cursor");
    return route.fulfill({
      json: {
        messages: previous
          ? [
              {
                id: "previous",
                role: "user",
                text: "Please include the project dates.",
                created_at: "2026-10-16T11:19:00Z",
                rated: false,
                truncated: false,
              },
            ]
          : [
              {
                id: "prompt",
                role: "user",
                text: "Summarize the project schedule.",
                created_at: "2026-10-16T11:20:00Z",
                rated: false,
                truncated: false,
              },
              {
                id: "reply-negative",
                role: "assistant",
                text: "The project is progressing as planned.",
                created_at: "2026-10-16T11:21:00Z",
                rated: true,
                truncated: false,
              },
            ],
        next_cursor: previous ? null : "previous-page",
        unavailable: false,
      },
    });
  });
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
        id: "rating-negative",
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
        id: "rating-positive",
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
        can_preview_threads: preview,
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

test("feedback overview labels failed refreshes and clears cached totals on authorization loss", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/admin/usage?tab=nyxbot");
  await expect(page.getByText("88%", { exact: true })).toBeVisible();
  await page
    .getByRole("button", { name: "Preview message thread" })
    .first()
    .click();
  await expect(page.getByText("Summarize the project schedule.")).toBeVisible();
  let status = 503;
  await page.route("**/api/v1/admin/assistant/feedback?**", (route) =>
    route.fulfill({
      status,
      json: { error: "unavailable", error_code: 1003, message: "Unavailable" },
    }),
  );
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(
    page.getByText(
      "Showing previously loaded feedback totals. Refresh failed.",
    ),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Unable to load assistant feedback. Matching totals are from the previous successful refresh.",
    ),
  ).toBeVisible();
  await expect(page.getByText("88%", { exact: true })).toBeVisible();
  status = 403;
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByText("Unable to load feedback totals.")).toBeVisible();
  await expect(page.getByText("88%", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Feedback is unavailable.")).toBeVisible();
  await expect(page.getByLabel("Matching feedback totals")).toHaveCount(0);
  await expect(page.getByText("Summarize the project schedule.")).toHaveCount(
    0,
  );
});

test("usage viewers see the full NyxBot tab", async ({ page }) => {
  await setup(page, false);
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
  await expect(
    page.getByRole("button", { name: "Preview message thread" }),
  ).toHaveCount(0);
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

for (const width of [1440, 390]) {
  test(`feedback review and anchored context are discoverable at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await setup(page);
    const requests: string[] = [];
    page.on("request", (r) => {
      if (r.url().includes("/thread")) requests.push(r.url());
    });
    await page.goto("/admin/usage?tab=nyxbot");
    const review = page.getByRole("heading", { name: "Recent feedback" });
    await expect(review).toBeVisible();
    await expect(page.getByLabel("Grant credits", { exact: true })).toHaveCount(
      0,
    );
    const model = page.getByRole("heading", {
      name: "Cost and billing by model",
    });
    expect((await review.boundingBox())!.y).toBeLessThan(
      (await model.boundingBox())!.y,
    );
    expect(requests).toHaveLength(0);
    await page
      .getByRole("button", { name: "Preview message thread" })
      .first()
      .click();
    const thread = page.getByRole("region", {
      name: "Feedback message thread",
    });
    await expect(thread).toContainText("Assistant · Rated reply");
    await expect(thread).toContainText("Summarize the project schedule.");
    await thread
      .getByRole("button", { name: "Load previous messages" })
      .click();
    await expect(thread.locator("ol > li").first()).toContainText(
      "Please include the project dates.",
    );
    await expect(thread.locator("ol > li")).toHaveCount(3);
    await expect(
      thread.getByRole("button", { name: "Load previous messages" }),
    ).toHaveCount(0);
    await thread.scrollIntoViewIfNeeded();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `test-results/feedback-thread-${width}.png`,
      animations: "disabled",
    });
    await page.getByRole("button", { name: "Hide message thread" }).click();
    await expect(thread).toHaveCount(0);
  });
}

for (const [width, theme, textScale] of [
  [1440, "dark", 1],
  [390, "light", 1],
  [320, "dark", 2],
] as const) {
  test(`grant credits stay complete by account and service at ${width}px ${theme} ${textScale}x`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await setup(page);
    await page.addInitScript(
      ({ theme, textScale }) =>
        localStorage.setItem(
          "nyxid.theme",
          JSON.stringify({
            state: { mode: theme, textScale, density: 1, motion: "reduce" },
            version: 2,
          }),
        ),
      { theme, textScale },
    );
    const requests: URLSearchParams[] = [];
    await page.route("**/api/v1/admin/usage/free-grants?**", (route) => {
      const params = new URL(route.request().url()).searchParams;
      requests.push(params);
      const currentPage = Number(params.get("page"));
      return route.fulfill({
        json: {
          window: { from: "2026-10-10T00:00:00Z", to: "2026-10-16T12:00:00Z" },
          generated_at: "2026-10-16T12:00:00Z",
          issued: "73.000000000001",
          used: "100.500000000001",
          interval: "day",
          trend: Array.from({ length: 7 }, (_, i) => ({
            start: `2026-10-${10 + i}T00:00:00Z`,
            end:
              i === 6 ? "2026-10-16T12:00:00Z" : `2026-10-${11 + i}T00:00:00Z`,
            issued: i === 0 ? "73.000000000001" : "0",
            used: i === 5 ? "100.500000000001" : "0",
          })),
          total_services: 51,
          service_page: Number(params.get("service_page")),
          services: [
            {
              service_slug:
                params.get("service_page") === "1" ? "nyxbot" : null,
              used:
                params.get("service_page") === "1" ? "5.000000000001" : "0.5",
            },
          ],
          grants: 68,
          total_accounts: 51,
          per_page: 50,
          page: currentPage,
          accounts: [
            {
              owner_id: currentPage === 1 ? "person-id" : "org-id",
              label:
                currentPage === 1
                  ? "Personal " + "longlabel".repeat(30)
                  : "Research organization",
              account_kind: currentPage === 1 ? "person" : "org",
              issued: "1.000000000001",
              used: "5.000000000001",
              grants: 1,
            },
          ],
        },
      });
    });
    await page.goto("/admin/usage");
    const report = page.getByLabel("Grant credits", { exact: true });
    await expect(
      report.getByText("Time and billing account filters · All services"),
    ).toBeVisible();
    await expect(
      report.getByText("Personal account · 1 grant"),
    ).not.toBeVisible();
    await expect(
      report.getByRole("img", { name: /Daily grant credits/ }),
    ).toBeVisible();
    await expect(report.getByTitle("100.500000000001").first()).toBeVisible();
    await expect(report.locator('dd[title="73.000000000001"]')).toBeVisible();
    await report
      .getByRole("button", { name: "View breakdown", exact: true })
      .click();
    await expect(report).toContainText("Personal account · 1 grant");
    await expect(report.getByTitle("1.000000000001")).toHaveText("1 credit");
    await report.getByRole("button", { name: "Next accounts" }).click();
    await expect(report).toContainText("Research organization");
    await expect(report).toContainText("Page 2 of 2");
    await expect(report.locator('dd[title="73.000000000001"]')).toBeVisible();
    expect(requests.at(-1)!.has("period")).toBe(false);
    await report.getByRole("tab", { name: "Service", exact: true }).click();
    await expect(report.getByText("nyxbot", { exact: true })).toBeVisible();
    await expect(report).toContainText("Page 1 of 2");
    await expect(report).toContainText(
      "Shared grants have no per-service granted amount",
    );
    await report.getByRole("button", { name: "Next services" }).click();
    await expect(
      report.getByText("Historical usage · service unavailable"),
    ).toBeVisible();
    expect(requests.at(-1)!.get("page")).toBe("2");
    await report.getByRole("tab", { name: "Account", exact: true }).click();
    await expect(report.getByText("Research organization")).toBeVisible();
    await report.getByText("View chart data", { exact: true }).click();
    const table = report.getByRole("table", {
      name: /Exact daily grant credits/,
    });
    await expect(table.getByText("73.000000000001")).toBeVisible();
    await expect(table.getByText("100.500000000001")).toBeVisible();
    await page.getByRole("button", { name: "Refresh", exact: true }).click();
    await expect.poll(() => requests.at(-1)!.get("page")).toBe("1");
    expect(requests.at(-1)!.get("period")).toBe("7d");
    await report
      .getByRole("button", { name: "View breakdown", exact: true })
      .click();
    await expect(report).toContainText("Personal account");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await report.screenshot({
      path: `test-results/grant-credits-${width}-${theme}-${textScale}.png`,
      animations: "disabled",
    });
  });
}
