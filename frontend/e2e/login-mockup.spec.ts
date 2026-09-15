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
  await page.locator('#keyList input[name="existing-key"]').first().check();
  await expect(page.locator("#grantKind")).toHaveText("Existing key");
  await expect(page.locator("#grantCustomize")).not.toHaveAttribute("open");
  await expect(page.locator("#existingPanel")).toBeHidden();
  await expect(page.locator("#serviceAccessRows details[open]")).toHaveCount(0);
  await page.locator("#serviceAccessRows summary").first().focus();
  await page.keyboard.press("Enter");
  await expect(
    page.locator("#serviceAccessRows .authorization-row-body").first(),
  ).toBeVisible();
  await page.locator("#serviceAccessRows summary").first().click();
  await expect(
    page.locator("#serviceAccessRows .authorization-row-body").first(),
  ).toBeHidden();
  await page.locator("#backToKeys").click();
  await page.locator("#createMatchingKey").click();
  await expect(
    page.locator("#serviceAccessRows .service-glyph svg").first(),
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
    page.locator("#serviceAccessRows .service-glyph").first(),
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
  await expect(page.locator("#grantCustomize")).not.toHaveAttribute("open");
  const gmail = page.locator('details[data-access-group="api-google-gmail"]');
  await expect(gmail.locator("summary")).toContainText("Choose account");
  await gmail.locator("summary").click();
  await gmail
    .getByRole("checkbox", {
      name: "Grant connection api-google-gmail-2",
      exact: true,
    })
    .check();
  await expect(gmail.locator("summary")).toContainText("api-google-gmail-2");
  await gmail.locator("summary").click();
  await expect(page.locator("#approveRestricted")).toBeEnabled();
  await page.locator("#grantCustomize > summary").click();
  await page.locator("#permissionSearch").fill("gmail");
  const extra = page.getByRole("option", {
    name: "Grant connection api-google-gmail-3",
    exact: true,
  });
  await extra.click();
  await expect(extra).toHaveAttribute("aria-selected", "true");
  await page.locator("#permissionSearch").press("Escape");
  await page.locator("#grantCustomize > summary").click();
  await expect(gmail.locator("summary")).toContainText("Matched + 1 extra");
  await gmail.locator("summary").click();
  await gmail
    .getByRole("checkbox", {
      name: "Grant connection api-google-gmail-3",
      exact: true,
    })
    .uncheck();
  await gmail.locator("summary").click();
  await expect(page.locator("#grantCustomize")).not.toHaveAttribute("open");
  await expect(page.locator("#serviceAccessRows details[open]")).toHaveCount(0);
  for (const [width, height] of [
    [1280, 900],
    [390, 844],
  ]) {
    await page.setViewportSize({ width: width!, height: height! });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    const card = await page.locator("#grantCard").boundingBox();
    expect(card!.height).toBeLessThan(600);
    await page.screenshot({
      path: info.outputPath(`authorization-${width}.png`),
      fullPage: true,
    });
  }
  await expect(page.locator("#grantKind")).toHaveText("Creates new key");
  await page.locator("#grantCustomize > summary").click();
  await page.locator("#permissionSearch").fill("gmail");
  const sendPermission = page.locator(
    '[data-permission="api-google-gmail::https://www.googleapis.com/auth/gmail.send"]',
  );
  await sendPermission.click();
  await expect(gmail.locator("summary")).toContainText("Check access");
  await expect(page.locator("#approveRestricted")).toBeDisabled();
  await sendPermission.click();
  await page.locator("#permissionSearch").press("Escape");
  await page.locator("#grantCustomize > summary").click();
  await expect(page.locator("#approveRestricted")).toBeEnabled();
  await page.locator("#approveRestricted").click();
  await expect(page.locator("#requestOutcome")).toBeVisible();
});
