import { expect, test } from "@playwright/test";
import { openAssistant } from "./helpers";
import { mockDashboard } from "./managed-onboarding-fixtures";

test("a specialist's guest preset and service overrides persist across reloads", async ({
  page,
}) => {
  await mockDashboard(page);
  await page.route("**/api/v1/assistant/nyxagent/machines?**", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/saved-logins**", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/triggers", (route) =>
    route.fulfill({ json: { triggers: [] } }),
  );
  await page.route("**/api/v1/approvals/requests?**", (route) =>
    route.fulfill({ json: { requests: [], total: 0 } }),
  );
  await page.route("**/api/v1/api-keys", (route) =>
    route.fulfill({
      json: {
        keys: [
          {
            id: "github-key",
            slug: "github",
            label: "GitHub",
            is_active: true,
          },
        ],
      },
    }),
  );
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /^researcher, specialist/ })
    .click();
  await page.getByRole("button", { name: "Agent details" }).click();
  const details = page.getByRole("dialog", { name: "Agent details" });
  const policy = details.getByRole("form", { name: "Guest policy" });
  const save = policy.getByRole("button", { name: "Save guest policy" });
  await expect(save).toBeDisabled();
  await expect(policy.getByRole("switch", { name: "Guest policy", exact: true })).not.toBeChecked();
  await expect(policy.getByRole("switch", { name: "Share skills with guests" })).toBeDisabled();
  await expect(
    policy.getByRole("switch", { name: "Share skills with guests" }),
  ).not.toBeChecked();
  await policy
    .getByRole("button", {
      name: "Same as me for everyday work; ask me for the rest",
    })
    .click();
  await expect(save).toBeEnabled();
  await expect(policy.getByRole("switch", { name: "Guest policy", exact: true })).toBeChecked();
  await expect(
    policy.getByRole("combobox", { name: "When a guest asks for more" }),
  ).toContainText("Ask me to approve");
  await expect(
    policy.getByRole("switch", { name: "Share skills with guests" }),
  ).toBeChecked();
  await save.click();
  await expect(save).toBeDisabled();

  await policy.getByRole("combobox", { name: "Default guest level" }).click();
  await page.getByRole("option", { name: "Read only", exact: true }).click();
  await save.click();
  await expect(save).toBeDisabled();
  const service = details
    .getByRole("list", { name: "Guest access" })
    .getByRole("combobox", { name: "github" });
  await expect(service).toContainText("Default (Look things up only)");
  await service.click();
  await page
    .getByRole("option", { name: "Use, but not change or delete", exact: true })
    .click();
  await details
    .getByRole("form", { name: "Grants" })
    .getByRole("button", { name: "Save grants" })
    .click();
  await expect(service).toContainText("Use, but not change or delete");
  await expect(
    details
      .getByRole("form", { name: "Grants" })
      .getByRole("button", { name: "Save grants" }),
  ).toBeDisabled();
  await page.reload();
  await page.getByRole("button", { name: "Agent details" }).click();
  await expect(
    policy.getByRole("combobox", { name: "Default guest level" }),
  ).toContainText("Read only");
  await expect(
    policy.getByRole("combobox", { name: "When a guest asks for more" }),
  ).toContainText("Ask me to approve");
  await expect(
    policy.getByRole("switch", { name: "Share skills with guests" }),
  ).toBeChecked();
  await expect(service).toContainText("Use, but not change or delete");
  await service.click();
  await page
    .getByRole("option", { name: "Default (Look things up only)", exact: true })
    .click();
  await details
    .getByRole("form", { name: "Grants" })
    .getByRole("button", { name: "Save grants" })
    .click();
  await expect(
    details
      .getByRole("form", { name: "Grants" })
      .getByRole("button", { name: "Save grants" }),
  ).toBeDisabled();
  await page.reload();
  await page.getByRole("button", { name: "Agent details" }).click();
  await expect(service).toContainText("Default (Look things up only)");
  await policy.getByRole("switch", { name: "Guest policy", exact: true }).click();
  await save.click();
  await expect(save).toBeDisabled();
  await page.reload();
  await page.getByRole("button", { name: "Agent details" }).click();
  await expect(policy.getByRole("switch", { name: "Guest policy", exact: true })).not.toBeChecked();
  await expect(policy.getByRole("combobox", { name: "Default guest level" })).toBeDisabled();
  await expect(policy.getByRole("combobox", { name: "Default guest level" })).toContainText("Read only");
  await expect(policy.getByRole("combobox", { name: "When a guest asks for more" })).toContainText("Ask me to approve");
  await expect(policy.getByRole("switch", { name: "Share skills with guests" })).toBeChecked();
  await expect(policy.getByRole("switch", { name: "Share skills with guests" })).toBeDisabled();
  await expect(service).toContainText("Default (Use, but not change or delete)");
});
