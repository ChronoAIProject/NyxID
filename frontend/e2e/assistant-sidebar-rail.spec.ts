import { expect, test } from "@playwright/test";
import { openAccountMenu, openAssistant, sendMessage } from "./helpers";

test("the sidebar hides to a rail of destinations and agents, and comes back", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 800 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();

  await page.getByRole("button", { name: "Hide sidebar" }).click();
  const rail = page.getByRole("navigation", { name: "Workspace and agents" });
  await expect(rail).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Agents and conversations" })).toHaveCount(0);
  for (const name of ["Home", "Automations", "Machines", "Plugins"]) {
    await expect(rail.getByRole(name === "Home" ? "button" : "link", { name, exact: true })).toBeVisible();
  }
  await expect(rail.getByRole("button", { name: "Create new agent" })).toBeVisible();

  // Hovering an agent lists its chats; picking one opens it.
  await rail.getByRole("button", { name: /^researcher/ }).hover();
  const chats = page.getByRole("list", { name: "researcher chats" });
  await expect(chats).toBeVisible();
  await rail.getByRole("button", { name: /^NyxBot/ }).first().hover();
  const nyxbotChats = page.getByRole("list", { name: "NyxBot chats" });
  await expect(nyxbotChats.getByRole("button", { name: "Use GitHub" })).toBeVisible();

  // Search and the account menu stay reachable.
  await expect(page.getByRole("button", { name: "Search", exact: true })).toBeVisible();
  await openAccountMenu(page);
  await expect(page.getByRole("menuitem", { name: "Settings", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");

  // The choice survives a reload; showing it again restores the full sidebar.
  await page.reload();
  await expect(page.getByRole("navigation", { name: "Workspace and agents" })).toBeVisible();
  await page.getByRole("button", { name: "Show sidebar" }).click();
  await expect(page.getByRole("navigation", { name: "Agents and conversations" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Workspace and agents" })).toHaveCount(0);
});
