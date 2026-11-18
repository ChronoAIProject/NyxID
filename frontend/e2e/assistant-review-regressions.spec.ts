import { expect, test } from "@playwright/test";
import { composerInput, openAssistant, sendMessage, SEEDED, stopButton } from "./helpers";

test("mobile title and Rename remain available after the thread header compacts", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, Array.from({ length: 120 }, (_, i) => `Transcript line ${i}`).join("\n"));
  await expect(page.getByText("Your connected services are ready.", { exact: false })).toBeVisible();
  const transcript = page.locator("[data-chat-transcript]");
  await transcript.evaluate((element) => {
    element.scrollTop = 400;
    element.dispatchEvent(new Event("scroll"));
  });
  await expect(page.getByRole("group", { name: "Conversation controls" })).toHaveAttribute("data-compact", "true");
  const rename = page.getByRole("button", { name: "Rename chat", exact: true });
  await expect(rename).toHaveCount(1);
  await expect(rename).toBeVisible();
  await rename.click();
  await page.getByRole("textbox", { name: "Chat title", exact: true }).fill("Mobile thread");
  await page.getByRole("button", { name: "Save title", exact: true }).click();
  await expect(rename).toHaveText("Mobile thread");
});

for (const width of [1440, 390]) {
  test(`NyxAgent draft keeps its title area through the first send at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    await page.goto("/assistant?mock=1&draft=true");
    const panels = page.locator("[data-chat-thread-panels]");
    const titleArea = width >= 768 ? panels : page.locator("header");
    await expect(titleArea.getByRole("heading", { name: "New chat", exact: true })).toBeVisible();
    await expect(page.getByRole("heading", { name: "New chat", exact: true })).toHaveCount(1);
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
    await expect(titleArea.getByRole("heading", { name: "List connected services", exact: true })).toBeVisible();
    expect((await panels.boundingBox())?.y).toBe(before?.y);
    await page.evaluate(() => (window as unknown as { releaseFirstSend: () => void }).releaseFirstSend());
    const rename = page.getByRole("button", { name: "Rename chat", exact: true });
    await expect(rename).toHaveCount(1);
    await expect(rename).toBeVisible();
    await expect(titleArea.getByRole("button", { name: "Rename chat", exact: true })).toBeVisible();
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
    await expect((width >= 768 ? page.locator("[data-chat-thread-panels]") : page.locator("header")).getByText("Renamed thread", { exact: true })).toBeVisible();
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
