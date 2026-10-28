import { expect, test } from "@playwright/test";
import { composerInput, openAssistant, sendMessage, SEEDED, stopButton } from "./helpers";

for (const width of [1440, 390]) {
  test(`NyxAgent draft keeps its title area through the first send at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    await page.goto("/assistant?mock=1&draft=true");
    const panels = page.locator("[data-chat-thread-panels]");
    await expect(panels.getByRole("heading", { name: "New chat", exact: true })).toBeVisible();
    await expect(page.getByRole("main").getByRole("heading", { name: "New chat", exact: true })).toHaveCount(1);
    const before = await panels.boundingBox();
    await page.evaluate(() => {
      const original = globalThis.__nyxidAssistantHttpMock!;
      globalThis.__nyxidAssistantHttpMock = async (request) => {
        if (request.endpoint === "/assistant/nyxagent/turns" && request.init.method === "POST") {
          await new Promise<void>((resolve) => {
            (window as unknown as { releaseFirstSend: () => void }).releaseFirstSend = resolve;
          });
        }
        return original(request);
      };
    });
    await composerInput(page).fill("List connected services");
    await composerInput(page).press("Enter");
    await expect.poll(() => page.evaluate(() => typeof (window as unknown as { releaseFirstSend?: () => void }).releaseFirstSend)).toBe("function");
    await expect(panels).toHaveCount(1);
    await expect(panels.getByRole("heading", { name: "List connected services", exact: true })).toBeVisible();
    expect((await panels.boundingBox())?.y).toBe(before?.y);
    await page.evaluate(() => (window as unknown as { releaseFirstSend: () => void }).releaseFirstSend());
    const rename = page.getByRole("button", { name: "Rename chat", exact: true });
    await expect(rename).toHaveCount(1);
    await expect(rename).toBeVisible();
    await expect(panels.getByRole("button", { name: "Rename chat", exact: true })).toBeVisible();
    expect((await panels.boundingBox())?.y).toBe(before?.y);
  });

  test(`one editable NyxAgent title at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    await sendMessage(page, "List connected services");
    await expect(page.getByRole("link", { name: "Connect GitHub" })).toBeVisible();
    await expect(stopButton(page)).not.toBeVisible();
    const rename = page.getByRole("button", { name: "Rename chat", exact: true });
    await expect(rename).toHaveCount(1);
    await expect(rename).toBeVisible();
    await expect(rename).toHaveText("List connected services");
    await rename.click();
    await page.getByRole("textbox", { name: "Chat title", exact: true }).fill("Renamed thread");
    await page.getByRole("button", { name: "Save title", exact: true }).click();
    await expect(rename).toHaveText("Renamed thread");
    await expect(page.locator("[data-chat-thread-panels]").getByText("Renamed thread", { exact: true })).toBeVisible();
  });

  test(`legacy and workspace titles and header actions stay visible at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await openAssistant(page, { conversation: SEEDED.github.id });
    await expect(page.getByRole("main").getByRole("heading", { name: SEEDED.github.title })).toBeVisible({ visible: width >= 768 });
    if (width < 768) await expect(page.locator("header").getByRole("heading", { name: SEEDED.github.title })).toBeVisible();
    await expect(page.getByRole("button", { name: "Mock scenarios", exact: true })).toBeVisible();
    if (width < 768) await page.getByRole("button", { name: "Open chats" }).click();
    await page.getByRole("link", { name: "Automations", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Automations", exact: true }).filter({ visible: true })).toHaveCount(1);
  });
}

test("legacy rail starts chats and opens previous chats only on click", async ({ page }) => {
  await openAssistant(page);
  await page.getByRole("button", { name: "Hide sidebar" }).click();
  const rail = page.getByRole("navigation", { name: "Workspace and agents" });
  await expect(rail.getByRole("button", { name: "New chat", exact: true })).toBeVisible();
  const previous = rail.getByRole("button", { name: "Previous chats" });
  await previous.hover();
  await expect(page.getByRole("list", { name: "Previous chats" })).toHaveCount(0);
  await previous.click();
  await page.getByRole("list", { name: "Previous chats" }).getByRole("button", { name: SEEDED.github.title }).click();
  await expect(page).toHaveURL(new RegExp(`c=${SEEDED.github.id}`));
  await rail.getByRole("button", { name: "New chat", exact: true }).click();
  await expect(page).not.toHaveURL(/c=/);
});
