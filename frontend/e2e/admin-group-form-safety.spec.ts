import { expect, test } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";

for (const viewport of [
  { width: 1440, height: 1000 },
  { width: 390, height: 844 },
]) {
  test(`group authorization conflict and sparse rename at ${String(viewport.width)}px`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await page.clock.setFixedTime(new Date("2026-09-17T00:00:00Z"));
    await mockDashboard(page);

    const roles = [
      { id: "role-a", name: "Role A", slug: "role-a" },
      { id: "role-b", name: "Role B", slug: "role-b" },
    ];
    let group = {
      id: "group-review",
      name: "Group A",
      slug: "group-a",
      description: "Saved",
      roles: [roles[0]],
      parent_group_id: null,
      is_system: false,
      created_at: "2026-09-17",
      updated_at: "2026-09-17",
    };
    const updates: unknown[] = [];
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));

    await page.route("**/api/v1/admin/roles", (route) =>
      route.fulfill({ json: { roles } }),
    );
    await page.route("**/api/v1/admin/groups/group-review/members", (route) =>
      route.fulfill({ json: { members: [] } }),
    );
    await page.route(
      "**/api/v1/admin/groups/group-review",
      async (route) => {
        if (route.request().method() === "PUT") {
          const patch = route.request().postDataJSON() as Record<
            string,
            unknown
          >;
          updates.push(patch);
          group = { ...group, ...patch };
        }
        await route.fulfill({ json: group });
      },
    );

    await page.goto("/admin/groups/group-review");
    await page.getByRole("button", { name: "Edit", exact: true }).click();
    const editor = page.getByRole("dialog", {
      name: "Edit Group",
      exact: true,
    });
    const selection = editor.getByLabel("Roles", { exact: true });
    await expect(selection).toHaveValues(["role-a"]);
    await selection.selectOption(["role-a", "role-b"]);
    await editor
      .getByRole("button", { name: "Save Changes", exact: true })
      .click();

    const review = page.getByRole("dialog", {
      name: "Review changes",
      exact: true,
    });
    const confirm = review.getByRole("button", {
      name: "Confirm changes",
      exact: true,
    });
    await expect(confirm).toBeEnabled();
    expect(updates).toEqual([]);

    group = { ...group, roles: [] };
    await page.clock.setFixedTime(new Date("2026-09-17T00:02:00Z"));
    const refetch = page.waitForResponse(
      (response) =>
        new URL(response.url()).pathname ===
          "/api/v1/admin/groups/group-review" &&
        response.request().method() === "GET",
    );
    await page.evaluate(() => {
      Object.defineProperty(document, "visibilityState", {
        configurable: true,
        value: "hidden",
      });
      window.dispatchEvent(new Event("visibilitychange"));
      Object.defineProperty(document, "visibilityState", {
        configurable: true,
        value: "visible",
      });
      window.dispatchEvent(new Event("visibilitychange"));
    });
    await refetch;
    await expect(confirm).toBeDisabled();
    await expect(review.getByRole("alert")).toContainText(
      "Saved values changed while you were editing",
    );
    expect(updates).toEqual([]);

    await review.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(review).toHaveCount(0);
    await selection.selectOption(["role-a"]);
    await expect(selection).toHaveValues(["role-a"]);
    await editor.getByLabel("Name", { exact: true }).fill("Rename only");
    await editor
      .getByRole("button", { name: "Save Changes", exact: true })
      .click();
    await expect(confirm).toBeEnabled();
    await confirm.click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    expect(updates).toEqual([{ name: "Rename only" }]);
    expect(group.roles).toEqual([]);
    expect(errors).toEqual([]);
  });
}
