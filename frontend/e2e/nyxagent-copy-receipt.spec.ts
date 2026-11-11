import { test, expect } from "@playwright/test";
import { openAssistant, sendMessage, stopButton } from "./helpers";

test.beforeEach(async ({ page }) => {
  await page.route("**/api/v1/assistant/nyxagent/machines?**", (route) =>
    route.fulfill({ json: [] }),
  );
});

for (const width of [1440, 390]) {
  test(`copy writes readable text and rich HTML, preserving the exact-turn receipt after reload at ${width}px`, async ({ page, context }) => {
    await page.setViewportSize({ width, height: 900 });
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await openAssistant(page, { faults: { nyxagentEnabled: true, progressStallMs: 1800 } });
    const userBody = "Review **literal** formatting and receipt";
    await sendMessage(page, userBody);
    await expect(stopButton(page)).toBeVisible();
    await expect(page.getByRole("button", { name: "Copy message", exact: true })).toHaveCount(1);
    await expect(page.getByText("Formatting reviewed", { exact: true })).toBeVisible({ timeout: 8000 });
    await expect(stopButton(page)).not.toBeVisible();
    const details = page.getByRole("button", { name: /Turn details:.*1 allowed by Full access/ });
    await details.click();
    const receipt = page.getByRole("list", { name: "Permissions used" }).getByRole("listitem").filter({ hasText: "Formatting proposal approved" });
    await expect(receipt).toHaveCount(1);
    await expect(receipt).toContainText("Full access");
    await expect(page.getByText("hidden arguments", { exact: false })).toHaveCount(0);
    await page.getByRole("button", { name: "Copy message", exact: true }).last().focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("button", { name: "Copied", exact: true })).toBeVisible();
    const spacedParagraph = " inline  code  and keep  repeated spaces.";
    const expected = `Formatting reviewed\n\n${spacedParagraph}\n\nconst answer = 42;\n`;
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(expected);
    const rich = await page.evaluate(async () => {
      const items = await navigator.clipboard.read();
      const item = items.find((candidate) => candidate.types.includes("text/html"));
      return { types: item?.types, html: item ? await (await item.getType("text/html")).text() : "" };
    });
    expect(rich.types).toEqual(expect.arrayContaining(["text/plain", "text/html"]));
    expect(rich.html).toContain("<strong>Formatting reviewed</strong>");
    expect(rich.html).toMatch(/<pre[^>]*><code[^>]*>const answer = 42;/);
    expect(rich.html).not.toContain("Formatting proposal approved");
    expect(rich.html).not.toContain("hidden arguments");
    // Render the actual clipboard HTML in a paste target with no message CSS.
    // innerText exercises visible whitespace, including inline-code inheritance.
    const pasted = await page.evaluate((html) => {
      const target = document.createElement("div");
      target.style.cssText = "position:fixed;left:0;top:0;width:1000px";
      target.innerHTML = html;
      document.body.append(target);
      try {
        const paragraph = [...target.querySelectorAll("p")].find((node) => node.textContent?.includes("repeated spaces"))!;
        return {
          text: paragraph.innerText,
          whitespace: getComputedStyle(paragraph).whiteSpace,
          codeWhitespace: getComputedStyle(paragraph.querySelector("code")!).whiteSpace,
        };
      } finally { target.remove(); }
    }, rich.html);
    expect(pasted).toEqual({ text: spacedParagraph, whitespace: "pre-wrap", codeWhitespace: "pre-wrap" });
    await page.reload();
    await details.click();
    await expect(receipt).toHaveCount(1);
    await expect(page.getByText("Formatting reviewed", { exact: true })).toBeVisible();
    await expect.poll(() => page.evaluate(() => {
      const receipt = [...document.querySelectorAll('[aria-label="Permissions used"] > li')].find((el) => el.textContent?.includes("Formatting proposal approved"));
      const answer = [...document.querySelectorAll("strong")].find((el) => el.textContent === "Formatting reviewed");
      const row = receipt?.closest("article");
      return Boolean(answer && row?.contains(answer) &&
        receipt!.compareDocumentPosition(answer) & Node.DOCUMENT_POSITION_FOLLOWING);
    })).toBe(true);
    await page.getByRole("button", { name: "Copy message", exact: true }).first().click();
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(userBody);
  });
}

test("clipboard failure stays retryable in the real chat renderer", async ({ page }) => {
  await page.addInitScript(() => {
    let fail = true;
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
      writeText: () => fail ? (fail = false, Promise.reject(new Error("clipboard unavailable"))) : Promise.resolve(),
    } });
  });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
  const copy = page.getByRole("button", { name: "Copy message", exact: true }).last();
  await copy.click();
  await expect(page.getByRole("status").filter({ hasText: "Copy failed. Try Copy message again." })).toBeVisible();
  await expect(copy).toBeEnabled();
  await copy.click();
  await expect(page.getByRole("button", { name: "Copied", exact: true })).toBeVisible();
  await expect(page.getByText("Copy failed. Try Copy message again.")).toHaveCount(0);
});
