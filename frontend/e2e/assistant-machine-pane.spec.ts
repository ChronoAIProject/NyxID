import { expect, test, type Page } from "@playwright/test";
import { openAssistant, sendMessage } from "./helpers";

async function withDesktop(page: Page) {
  await page.route("**/api/v1/assistant/nyxagent/machines?*", (route) =>
    route.fulfill({
      json: [{ node_id: "node-1", session_id: "s1", conversation_id: null, status: "agent", reason: null, display: "secure" }],
    }),
  );
  await page.route("**/api/v1/assistant/nyxagent/machines/*/contexts", (route) => route.fulfill({ json: [] }));
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
}

test("an open desktop docks beside the chat behind a draggable divider", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await withDesktop(page);
  const column = page.getByRole("complementary", { name: "Machine" });
  await expect(column).toBeVisible();
  await expect(column.getByRole("region", { name: "Live machine desktop" })).toBeVisible();
  // One machine: no machine row, just the view controls on a single line.
  await expect(column.getByRole("tablist")).toHaveCount(0);
  await expect(column.getByRole("button", { name: "Secure browser" })).toBeVisible();
  // Docked, so never minimised; fullscreen and pop-out stay available.
  await expect(column.getByRole("button", { name: "Minimise desktop" })).toHaveCount(0);
  await expect(column.getByRole("button", { name: "Expand desktop" })).toBeVisible();

  const main = (await page.getByRole("main").boundingBox())!;
  expect((await column.boundingBox())!.width).toBeCloseTo(main.width * 0.5, -1);
  // Two resizers stand at the top: the chat width grips and the split divider.
  await expect(page.getByRole("separator", { name: "Resize chat content from right" })).toBeAttached();
  const divider = page.getByRole("separator", { name: "Resize machine column" });
  await expect(divider).toHaveAttribute("aria-valuenow", "50");
  const grip = (await divider.boundingBox())!;
  await page.mouse.move(grip.x + grip.width / 2, grip.y + 10);
  await page.mouse.down();
  await page.mouse.move(grip.x + grip.width / 2 - main.width * 0.15, grip.y + 10, { steps: 8 });
  await page.mouse.up();
  await expect(divider).toHaveAttribute("aria-valuenow", "65");
  await expect.poll(async () => Math.round((await column.boundingBox())!.width)).toBe(Math.round(main.width * 0.65));
  // Keys move it too, within bounds; double-click resets.
  await divider.focus();
  await page.keyboard.press("End");
  await expect(divider).toHaveAttribute("aria-valuenow", "25");
  await page.keyboard.press("ArrowLeft");
  await expect(divider).toHaveAttribute("aria-valuenow", "27");
  await divider.dblclick();
  await expect(divider).toHaveAttribute("aria-valuenow", "50");
  await page.keyboard.press("Home");
  await expect(divider).toHaveAttribute("aria-valuenow", "75");
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await expect(divider).toHaveAttribute("aria-valuenow", "65");

  // The pill's Machine button hides and shows it; the choice is per thread.
  const toggle = page.getByRole("group", { name: "Conversation controls" }).getByRole("button", { name: "Hide machine" });
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await toggle.click();
  await expect(column).toHaveCount(0);
  await page.reload();
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
  await expect(page.getByRole("complementary", { name: "Machine" })).toHaveCount(0);
  await page.getByRole("button", { name: "Show machine" }).click();
  await expect(page.getByRole("complementary", { name: "Machine" })).toBeVisible();
  // The width choice persisted too.
  await expect.poll(async () => Math.round((await page.getByRole("complementary", { name: "Machine" }).boundingBox())!.width)).toBe(Math.round(main.width * 0.65));
});

test("narrow screens keep the desktop inline above the transcript", async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 860 });
  await withDesktop(page);
  await expect(page.getByRole("complementary", { name: "Machine" })).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Live machine desktop" })).toBeVisible();
  await page.getByRole("button", { name: "Hide machine" }).click();
  await expect(page.getByRole("region", { name: "Live machine desktop" })).toHaveCount(0);
});

test("threads without a desktop show no machine button", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page.route("**/api/v1/assistant/nyxagent/machines?*", (route) => route.fulfill({ json: [] }));
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
  await expect(page.getByRole("button", { name: /(Show|Hide) machine/ })).toHaveCount(0);
  await expect(page.getByRole("complementary", { name: "Machine" })).toHaveCount(0);
});

test("several machines share the column as tabs, one streaming at a time", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 860 });
  // Names come from the demo machine list (node-0001 prod-us-east, node-0002 staging-eu).
  await page.route("**/api/v1/assistant/nyxagent/machines?*", (route) =>
    route.fulfill({
      json: [
        { node_id: "node-0001", session_id: "s1", conversation_id: null, status: "agent", reason: null, display: "secure" },
        { node_id: "node-0002", session_id: "s2", conversation_id: null, status: "owner", reason: "Approve the sign-in", display: "secure" },
      ],
    }),
  );
  await page.route("**/api/v1/assistant/nyxagent/machines/*/contexts", (route) => route.fulfill({ json: [] }));
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();

  const column = page.getByRole("complementary", { name: "Machine" });
  const tabs = column.getByRole("tablist", { name: "Machines" });
  await expect(tabs.getByRole("tab")).toHaveText(["prod-us-east", "staging-eu"]);
  await expect(tabs.getByRole("tab", { name: /staging-eu/ }).getByRole("img", { name: "Needs you" })).toBeVisible();
  await expect(column.getByRole("region", { name: "Live machine desktop" })).toHaveCount(1);
  await expect(tabs.getByRole("tab", { name: /prod-us-east/ })).toHaveAttribute("aria-selected", "true");
  await expect(column.getByText("NyxBot needs you on this machine")).toHaveCount(0);

  await tabs.getByRole("tab", { name: /staging-eu/ }).click();
  await expect(tabs.getByRole("tab", { name: /staging-eu/ })).toHaveAttribute("aria-selected", "true");
  await expect(column.getByText("NyxBot needs you on this machine: Approve the sign-in")).toBeVisible();
});
