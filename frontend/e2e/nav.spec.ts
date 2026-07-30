import { expect, test, type Page } from "@playwright/test";
import { openAssistant, SEEDED } from "./helpers";

const destinations = [
  { label: "Plugins", pathname: "/assistant/plugins" },
  { label: "Approvals", pathname: "/assistant/approvals" },
  { label: "Studio", pathname: "/dashboard" },
] as const;

const sources = [
  { label: "bare", conversation: undefined },
  { label: "selected chat", conversation: SEEDED.stripe.id },
] as const;

async function expectPathname(page: Page, pathname: string): Promise<void> {
  await expect.poll(() => new URL(page.url()).pathname).toBe(pathname);
}

test.describe("assistant sidebar navigation", () => {
  for (const source of sources) {
    for (const destination of destinations) {
      test(`desktop: ${source.label} opens ${destination.label} on the first click`, async ({
        page,
      }) => {
        await openAssistant(page, { conversation: source.conversation });

        await page.getByRole("link", { name: destination.label }).click();

        await expectPathname(page, destination.pathname);
      });

      test(`mobile drawer: ${source.label} opens ${destination.label} on the first click`, async ({
        page,
      }) => {
        await page.setViewportSize({ width: 390, height: 844 });
        await openAssistant(page, { conversation: source.conversation });
        await page.getByRole("button", { name: "Open chats" }).click();
        await expect(
          page.getByRole("button", { name: "Close chats" }).first(),
        ).toBeVisible();

        await page.getByRole("link", { name: destination.label }).click();

        await expectPathname(page, destination.pathname);
        await expect(
          page.getByRole("button", { name: "Close chats" }),
        ).toHaveCount(0);
      });
    }
  }
});
