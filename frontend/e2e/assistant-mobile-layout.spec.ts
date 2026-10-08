import { expect, test, type Page } from "@playwright/test";
import { openAssistant, sendMessage } from "./helpers";

test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

async function viewport(page: Page, height: number, offsetTop = 0) {
  await page.evaluate(({ height, offsetTop }) => {
    Object.defineProperties(window.visualViewport!, {
      height: { configurable: true, value: height },
      offsetTop: { configurable: true, value: offsetTop },
    });
    window.visualViewport!.dispatchEvent(new Event("resize"));
    window.visualViewport!.dispatchEvent(new Event("scroll"));
  }, { height, offsetTop });
}

for (const engine of ["actor", "NyxAgent", "group"] as const) {
  test(`mobile composer and transcript handle keyboard resize (${engine})`, async ({ page }) => {
    await openAssistant(page, { faults: { nyxagentEnabled: engine !== "actor" } });
    if (engine === "group") {
      await page.getByRole("region", { name: "Groups", exact: true }).getByRole("button", { name: "New group" }).click();
      const dialog = page.getByRole("dialog", { name: "New group", exact: true });
      await dialog.getByRole("textbox", { name: "Name", exact: true }).fill("Mobile group");
      await dialog.getByRole("button", { name: "Create group", exact: true }).click();
      await expect(page).toHaveURL(/g=nyxg-/);
    }
    await sendMessage(page, Array.from({ length: 120 }, (_, i) => `Mobile transcript line ${i}`).join("\n"));
    if (engine === "group") await expect(page.getByRole("article", { name: "Message from NyxBot", exact: true })).toBeVisible();
    if (engine === "NyxAgent") await expect(page.getByText("Your connected services are ready.", { exact: false })).toBeVisible();
    const send = page.getByRole("button", { name: "Send message", exact: true });
    await expect(send).toBeVisible();
    const input = page.locator("textarea");
    await input.fill("A longer mobile draft\nwith multiple lines\nand the send control still reachable");
    expect(await input.evaluate((el) => parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(16);
    const scroller = page.locator("main div.flex-1.overflow-y-auto").first();
    const distance = () => scroller.evaluate((el) => el.scrollHeight - el.clientHeight - el.scrollTop);
    await expect.poll(distance).toBeLessThan(2);
    const band = page.locator("[data-composer-band]");
    const checkBounds = async (height: number, offsetTop = 0) => {
      for (const control of [band, send]) {
        const bounds = await control.boundingBox();
        expect(bounds).not.toBeNull();
        expect(bounds!.x).toBeGreaterThanOrEqual(0);
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
        expect(bounds!.y).toBeGreaterThanOrEqual(offsetTop);
        expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(offsetTop + height + 1);
      }
      expect(await page.evaluate(() => window.scrollY)).toBe(0);
    };
    await checkBounds(844);
    // Desktop automation does not open an OS keyboard. Model its viewport events.
    await viewport(page, 420, 60);
    await checkBounds(420, 60);
    await expect.poll(distance).toBeLessThan(2);
    await viewport(page, 844);
    await expect.poll(distance).toBeLessThan(2);

    // Resizing must not drag a reader away from older messages.
    await scroller.evaluate((el) => { el.scrollTop = 150; el.dispatchEvent(new Event("scroll")); });
    await viewport(page, 420);
    await expect.poll(() => scroller.evaluate((el) => el.scrollTop)).toBe(150);
    await checkBounds(420);
    await scroller.evaluate((el) => { el.scrollTop = 0; el.dispatchEvent(new Event("scroll")); });
    const headerTop = (await page.locator("header").boundingBox())!.y;
    await viewport(page, 844);
    await scroller.hover({ position: { x: 20, y: 20 } });
    await page.mouse.wheel(0, -1000);
    await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
    expect((await page.locator("header").boundingBox())!.y).toBe(headerTop);
    await viewport(page, 844);

    await page.getByRole("button", { name: "User menu", exact: true }).click();
    await page.getByRole("menuitem", { name: "Open Studio", exact: true }).click();
    await expect(page).toHaveURL(/\/dashboard/);
    await expect.poll(() => page.evaluate(() => [document.body.style.overflow, document.documentElement.style.overscrollBehavior])).toEqual(["", ""]);
  });
}
