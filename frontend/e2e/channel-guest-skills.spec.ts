import { expect, test } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import type { GuestSkillAttachment } from "../src/schemas/channel-guest-skills";

test("maintainer approves a current pinned skill for private guest replies", async ({ page }) => {
  await mockDashboard(page);
  const botId = "managed-whatsapp";
  const api = "**/api/v1/channel-bots/" + botId;
  await page.route(api + "/guest-setup", (route) =>
    route.fulfill({
      json: {
        available: true, enabled: true, bot_revision: 1,
        settings: {
          revision: 0, policy_revision: 0, draft: null, applied: null,
          effective_rule_count: 0, verified_guests: [], verified_rooms: [],
          sheets_connections: [], sources: [],
        },
      },
    }),
  );
  const skill = {
    source: "ornn",
    skill_id: "1f11c47b-7b94-483d-8d3a-4af8f7055e04",
    name: "Help Center",
    version: "1.2",
    sha256: "a".repeat(64),
    dependencies: [],
  };
  let revision = 0;
  let attachments: GuestSkillAttachment[] = [];
  await page.route(api + "/guest-skills", (route) => {
    if (route.request().method() === "PUT") {
      const input = route.request().postDataJSON();
      expect(input).toEqual({
        expected_revision: 0,
        skills_revision: 3,
        attachments: [{
          skill_id: skill.skill_id,
          audiences: ["private"],
          all_requests_approved: true,
        }],
      });
      attachments = input.attachments as GuestSkillAttachment[];
      revision = 1;
    }
    return route.fulfill({
      json: { revision, skills_revision: 3, available: [skill], attachments },
    });
  });

  await page.goto("/channel-bots/" + botId);
  const skillEntry = page.getByRole("group", { name: "Help Center 1.2" });
  const privateAudience = skillEntry.getByRole("checkbox", { name: "Private replies" });
  const sharedAudience = skillEntry.getByRole("checkbox", { name: "Shared replies" });
  const standingApproval = skillEntry.getByRole("checkbox", {
    name: "Approve reads of this exact version and its pinned dependencies under All Requests",
  });
  await expect(privateAudience).toBeChecked();
  await expect(sharedAudience).toBeChecked();
  await expect(standingApproval).not.toBeChecked();
  await sharedAudience.click();
  await standingApproval.click();
  const saved = page.waitForResponse(
    (response) => response.url().endsWith("/guest-skills") && response.request().method() === "PUT",
  );
  await page.getByRole("button", { name: "Save skill access" }).click();
  expect((await saved).ok()).toBe(true);
  await page.reload();
  await expect(privateAudience).toBeChecked();
  await expect(sharedAudience).not.toBeChecked();
  await expect(standingApproval).toBeChecked();
});
