import { expect, test } from "@playwright/test";
import { writeFileSync } from "node:fs";
import { openAssistant, SEEDED } from "./helpers";

test("a historical confirm-gate snapshot stays readable and sends no plan.resolve", async (
  { page },
  testInfo,
) => {
  const network: string[] = [];
  page.on("request", (request) => {
    const body = request.postData() ?? "";
    network.push(`${request.method()} ${request.url()} ${body}`);
  });

  await openAssistant(page, { conversation: SEEDED.taskPlan.id });
  const plan = page.getByRole("region", { name: "Task plan" });
  await expect(plan).toBeVisible({ timeout: 20_000 });
  await expect(plan.getByText("Publish a weekly update")).toBeVisible();
  await expect(plan.getByText("confirm / pending")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Confirm plan" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Reject plan" }),
  ).toHaveCount(0);

  const screenshotPath = testInfo.outputPath("task-plan-after.png");
  await plan.screenshot({ path: screenshotPath });
  await testInfo.attach("task-plan-after", {
    path: screenshotPath,
    contentType: "image/png",
  });
  const networkPath = testInfo.outputPath("task-plan-network.log");
  writeFileSync(networkPath, `${network.join("\n")}\n`);
  await testInfo.attach("task-plan-network", {
    path: networkPath,
    contentType: "text/plain",
  });

  expect(
    network.filter((entry) => entry.includes("plan.resolve")),
  ).toEqual([]);
});
