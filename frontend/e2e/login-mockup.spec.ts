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

test("mockup selects a connected account in the permission dropdown and completes the grant", async ({
  page,
}, info) => {
  await openMockup(page);
  await page.locator('[data-step="2"]').click();
  await expect(page.locator('[data-mode="full"] svg')).toBeVisible();
  await expect(page.locator('[data-mode="agent"] svg')).toBeVisible();
  await page.screenshot({
    path: info.outputPath("access-cards.png"),
    fullPage: true,
  });
  await page.locator("#continueScope").click();
  await page.locator("#createMatchingKey").click();
  await expect(page.locator("#approveRestricted")).toBeDisabled();
  await page.locator("#permissionSearch").fill("gmail");
  const account = page.getByRole("option", {
    name: "Grant connection api-google-gmail-2",
    exact: true,
  });
  await expect(account).toHaveAttribute("aria-selected", "false");
  await account.click();
  await expect(account).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("#serviceCount")).toHaveText(
    "2 connections selected",
  );
  await page.locator("#permissionSearch").press("Escape");
  await page.getByRole("button", { name: "Add service", exact: true }).click();
  await expect(page.locator("#permissionSearch")).toBeFocused();
  await expect(page.locator("#permissionSearch")).toHaveValue("");
  await page.locator("#permissionSearch").fill("gmail");
  await page
    .getByRole("option", {
      name: "Grant connection api-google-gmail-3",
      exact: true,
    })
    .click();
  await page.locator("#permissionSearch").press("Escape");
  await expect(page.locator("#serviceCount")).toHaveText(
    "3 connections selected",
  );
  await expect(page.locator("#resourceList")).toContainText(
    "Extra permissions included · 1",
  );
  await page
    .getByRole("button", {
      name: "Remove connection api-google-gmail-3",
      exact: true,
    })
    .click();
  await expect(page.locator("#serviceCount")).toHaveText(
    "2 connections selected",
  );
  await page.locator("#permissionSearch").fill("gmail");
  for (const [width, height] of [
    [1280, 900],
    [390, 844],
  ]) {
    await page.setViewportSize({ width: width!, height: height! });
    await page.locator("#permissionSearch").scrollIntoViewIfNeeded();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: info.outputPath(`combined-access-${width}.png`),
      fullPage: true,
    });
  }
  await page.locator("#permissionSearch").press("Escape");
  await expect(page.locator("#resourceList")).toContainText("Gmail");
  await expect(page.locator("#resourceList [data-catalog-only]")).toHaveCount(
    0,
  );
  await expect(page.locator("#approveRestricted")).toBeEnabled();
  await page.locator("#permissionSearch").fill("gmail");
  const sendPermission = page.locator(
    '[data-permission="api-google-gmail::https://www.googleapis.com/auth/gmail.send"]',
  );
  await sendPermission.click();
  await expect(page.locator("#resourceList")).toContainText(
    "Missing requested permissions",
  );
  await expect(page.locator("#approveRestricted")).toBeDisabled();
  await sendPermission.click();
  await page.locator("#permissionSearch").press("Escape");
  await expect(page.locator("#approveRestricted")).toBeEnabled();
  await page.locator("#approveRestricted").click();
  await expect(page.locator("#requestOutcome")).toBeVisible();
});
