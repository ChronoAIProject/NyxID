import { test, expect } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";

for (const width of [1440, 390]) {
  test(`flag organization, sunset and history at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 1000 });
    await mockDashboard(page);
    const makeFlag = (key: string, sunset: boolean) => ({
      key,
      description: `Controls ${key}`,
      code_description: `Controls ${key}`,
      custom_description: null,
      owner: "Assistant team",
      group: "Assistant",
      tags: ["voice"],
      sunset,
      can_edit: true,
      default_enabled: false,
      global_override: true,
      org_overrides: [],
      user_overrides: [],
    });
    const flags = [
      makeFlag("assistant:voice", false),
      makeFlag("assistant:legacy", true),
    ];
    const writes: unknown[] = [];
    await page.route("**/api/v1/admin/feature-flags**", async (route) => {
      const request = route.request();
      const path = decodeURIComponent(new URL(request.url()).pathname);
      const flag = flags.find((flag) => path.includes(flag.key));
      let body: unknown = { flags };
      if (path.endsWith("/history"))
        body = {
          entries: [
            {
              id: "change-1",
              actor_id: "user-1",
              actor_name: "Alex Chen",
              actor_email: "alex@example.test",
              event_type: "admin_feature_flag_override_set",
              details: {
                target_kind: "global",
                before: false,
                after: true,
                enabled: true,
              },
              created_at: "2026-10-29T01:00:00Z",
            },
          ],
          next_cursor: null,
          legacy_ambiguous_omitted: false,
          origin: {
            created_by: "user-2",
            creator_name: "Sam Rivera",
            created_at: "2026-10-28T00:00:00Z",
          },
        };
      else if (path.endsWith("/metadata") && request.method() === "PATCH") {
        const patch = request.postDataJSON();
        writes.push(patch);
        Object.assign(flag!, patch);
        body = flag;
      }
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(body),
      });
    });
    await page.goto("/admin/feature-flags");
    const active = page.getByTestId("flag-assistant:voice");
    const legacy = page.getByTestId("flag-assistant:legacy");
    await expect(active).toBeVisible();
    await expect(legacy).toBeHidden();
    await page.getByRole("tab", { name: /^Sunset/ }).click();
    await expect(legacy).toBeVisible();
    await expect(active).toBeHidden();
    await page.getByRole("tab", { name: /^Active/ }).click();
    await page.getByLabel("Filter by tag").selectOption("voice");
    await active.getByRole("button", { name: /assistant:voice/ }).click();
    await active.getByLabel("Group", { exact: true }).fill("Voice experiences");
    await page.getByLabel("Group flags by").selectOption("owner");
    await expect(active.getByLabel("Group", { exact: true })).toHaveValue(
      "Voice experiences",
    );
    await active.getByRole("button", { name: "Save organization" }).click();
    await expect(page.getByRole("dialog")).toContainText("Voice experiences");
    expect(writes).toHaveLength(0);
    await page.getByRole("button", { name: "Confirm changes" }).click();
    await expect.poll(() => writes).toEqual([{ group: "Voice experiences" }]);
    await active.getByRole("tab", { name: "History", exact: true }).click();
    await expect(active).toContainText("Sam Rivera");
    await expect(active).toContainText("Alex Chen");
    await expect(active).toContainText(
      "Global · all users · Disabled → Enabled",
    );
    await active.scrollIntoViewIfNeeded();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath(`flags-${width}-history.png`),
      fullPage: true,
    });
  });
}
