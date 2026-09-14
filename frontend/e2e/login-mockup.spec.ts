import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";

async function openMockup(page: Page, hostileIconText = false) {
  let html = await readFile(
    new URL("../../docs/mockups/login-consolidated.html", import.meta.url),
    "utf8",
  );
  if (hostileIconText) {
    const setup = `<script>
      const iconText = '<img src="missing-icon" onerror="document.body.dataset.iconExecuted = 1">';
      const oldSnapshot = document.getElementById('serviceIconSnapshot');
      if (oldSnapshot) {
        const icons = JSON.parse(oldSnapshot.textContent);
        for (const slug of Object.keys(icons)) icons[slug] = iconText;
        oldSnapshot.textContent = JSON.stringify(icons);
      }
      document.querySelectorAll('template[data-service-icon]').forEach(template => {
        template.content.textContent = iconText;
      });
    </script>`;
    const script = html.lastIndexOf("<script>");
    html = html.slice(0, script) + setup + html.slice(script);
  }
  await page.route("**/docs/mockups/login-consolidated.html*", (route) =>
    route.fulfill({ contentType: "text/html", body: html }),
  );
  await page.route("**/frontend/public/nyxid-coloured-icon.svg", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      path: fileURLToPath(
        new URL("../public/nyxid-coloured-icon.svg", import.meta.url),
      ),
    }),
  );
  await page.route("**/missing-icon", (route) =>
    route.fulfill({ status: 404 }),
  );
  await page.goto("/docs/mockups/login-consolidated.html");
  await page.goto(
    (await page.locator("#sampleKeyExample").getAttribute("href")) as string,
  );
  await page.locator("#verifyContinue").click();
  await page.locator("#continueScope").click();
}

test("mockup preserves service glyphs and preloaded permission choices", async ({
  page,
}) => {
  await openMockup(page);
  await expect(page.locator("#keyList .key-option").first()).toHaveAttribute(
    "data-match",
    "exact",
  );
  await expect(
    page.locator("#keyList .service-glyph svg").first(),
  ).toBeVisible();
  await expect(
    page.locator("#permissionChips .service-glyph svg").first(),
  ).toBeVisible();
  await expect
    .poll(() =>
      page
        .locator("#permissionChips .service-glyph img")
        .evaluate(
          (image: HTMLImageElement) => image.complete && image.naturalWidth > 0,
        ),
    )
    .toBe(true);
  await page.locator("#permissionSearch").fill("github");
  await expect(
    page.locator("#permissionOptions .service-glyph svg").first(),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.locator("#createMatchingKey").click();
  await expect(
    page.locator("#resourceList .service-glyph svg").first(),
  ).toBeVisible();
  await page.screenshot({
    path: test.info().outputPath("mockup-scopes.png"),
    fullPage: true,
  });
});

test("mockup never interprets icon snapshot text as active HTML", async ({
  page,
}) => {
  await openMockup(page, true);
  await expect(page.locator("body")).not.toHaveAttribute(
    "data-icon-executed",
    "1",
  );
  await expect(
    page.locator('.service-glyph img[src="missing-icon"]'),
  ).toHaveCount(0);
  await expect(page.locator("#keyList .service-glyph").first()).toContainText(
    "<img",
  );
  await page.locator("#createMatchingKey").click();
  await expect(
    page.locator("#resourceList .service-glyph").first(),
  ).toContainText("<img");
  await expect(page.locator("body")).not.toHaveAttribute(
    "data-icon-executed",
    "1",
  );
});
