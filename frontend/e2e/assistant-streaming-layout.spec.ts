import { expect, test } from "@playwright/test";
import { openAssistant, sendMessage, stopButton } from "./helpers";

for (const width of [390, 1280]) {
  test(`a live reply keeps long links and a paused Markdown tail visible at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await openAssistant(page, { faults: { nyxagentEnabled: true, progressStallMs: 30_000 } });
    const url = `https://images.example.com/${"attachment".repeat(35)}.jpeg`;
    const parts = [`Retrieving [the attachment](${url}).\n\n`, `${"identifier".repeat(35)}\n\n`, "I am **checking the image"];
    await page.evaluate((parts) => {
      const original = globalThis.__nyxidAssistantHttpMock!;
      globalThis.__nyxidAssistantHttpMock = async (request) => {
        const response = await original(request);
        if (!response?.body || !request.endpoint.endsWith("/turns")) return response;
        // Keep real fixture admission and settlement; change only the streamed
        // text so this exercises the actual transport and message renderer.
        const decoder = new TextDecoder();
        const encoder = new TextEncoder();
        let cursor = 0;
        return new Response(response.body.pipeThrough(new TransformStream({
          async transform(chunk, controller) {
            const event = JSON.parse(decoder.decode(chunk).slice("data: ".length));
            const emit = (text?: string) => controller.enqueue(encoder.encode(
              `data: ${JSON.stringify({ ...event, cursor: ++cursor, ...(text === undefined ? {} : { text }) })}\n\n`,
            ));
            if (event.event !== "block.delta") {
              emit();
              return;
            }
            for (const part of parts) {
              await new Promise((resolve) => setTimeout(resolve, 150));
              emit(part);
            }
          },
        })), { headers: response.headers });
      };
    }, parts);
    await sendMessage(page, `Forward this image: ${url}`);
    await expect(stopButton(page)).toBeVisible();
    const caret = page.locator("[data-streaming-caret]");
    await expect(caret).toHaveCount(1);
    const answer = caret.locator("..");
    await expect(answer).toContainText("I am **checking the image", { timeout: 3000 });
    const row = caret.locator('xpath=ancestor::div[starts-with(@id,"message-")]');
    const user = page.getByText(`Forward this image: ${url}`, { exact: true });
    for (const content of [user, answer, row]) {
      await expect.poll(() => content.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(1);
      const bounds = (await content.boundingBox())!;
      expect(bounds.x).toBeGreaterThanOrEqual(0);
      expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
    }
    const scroller = row.locator('xpath=ancestor::div[contains(@class,"overflow-y-auto")]').first();
    await expect.poll(() => scroller.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(1);
    await expect(stopButton(page)).toBeVisible();
    await page.screenshot({ path: `test-results/assistant-streaming-${width}.png` });
  });
}
