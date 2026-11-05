import { expect, test } from "@playwright/test";
import { composerInput, openAssistant, sendMessage } from "./helpers";

test("the latest failed reply is dismissable above the composer with its durable trace", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page.evaluate(() => {
    const original = globalThis.__nyxidAssistantHttpMock;
    globalThis.__nyxidAssistantHttpMock = async (request) => {
      const response = await original?.(request);
      if (response?.ok && /^\/assistant\/nyxagent\/conversations\/[^/?]+(?:\?|$)/.test(request.endpoint) && request.init.method === "GET") {
        const body = await response.json();
        const reply = body.messages?.filter((message: { role: string }) => message.role === "assistant").at(-1);
        if (reply) {
          reply.status = "failed";
          reply.error_code = "idle_timeout";
          reply.turn_id = "9ad6d0fa-bb8f-489e-8fdd-f8e34960921b";
          reply.recovery = { status: "available", reason: "available", text: "Check my services", attachments: [] };
        }
        return Response.json(body);
      }
      return response;
    };
  });
  await sendMessage(page, "Check my services");
  const notice = page.getByRole("alert").filter({ hasText: "The assistant stopped responding. Check what it already did before asking it to continue." });
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("Trace ID: 9ad6d0fa-bb8f-489e-8fdd-f8e34960921b");
  await expect(page.locator("[data-composer-dock]").getByRole("alert")).toContainText("Trace ID:");
  expect((await notice.boundingBox())!.y + (await notice.boundingBox())!.height).toBeLessThanOrEqual((await composerInput(page).boundingBox())!.y);
  // Keep retry beside the reply, with just one visible docked failure message.
  await expect(page.getByRole("button", { name: "Retry", exact: true })).toBeVisible();
  await expect(page.getByText("The assistant stopped responding. Check what it already did before asking it to continue.", { exact: false })).toHaveCount(1);
  await page.evaluate(() => {
    const original = globalThis.__nyxidAssistantHttpMock;
    globalThis.__nyxidAssistantHttpMock = (request) => {
      if (request.endpoint.endsWith("/recovery")) {
        return Response.json({ status: "available", reason: "available", text: "Check my services", attachments: [] });
      }
      if (request.endpoint.endsWith("/turns")) {
        return Response.json({ message: "private upstream diagnostic" }, {
          status: 503,
          headers: { "X-NyxID-Trace-Id": "e48e2e01-6c09-4bb7-b5d2-2ab023038dea" },
        });
      }
      return original?.(request);
    };
  });
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  const retryToast = page.locator("[data-sonner-toast]").filter({ hasText: "Could not retry your message" });
  await expect(retryToast).toContainText("NyxBot is temporarily unavailable. Try again shortly.");
  await expect(retryToast).not.toContainText("Trace ID:");
  const retryNotice = page.getByRole("alert").filter({ hasText: "Trace ID: e48e2e01-6c09-4bb7-b5d2-2ab023038dea" });
  await expect(retryNotice).toBeVisible();
  await expect(page.locator("[data-composer-dock]").getByRole("alert")).toContainText("e48e2e01");
  await expect(page.getByText("Trace ID: e48e2e01-6c09-4bb7-b5d2-2ab023038dea", { exact: false })).toHaveCount(1);
  await retryNotice.getByRole("button", { name: "Dismiss", exact: true }).click();
  await expect(retryNotice).toHaveCount(0);
});

for (const mobile of [false, true]) {
  test(`NyxBot keeps descriptive toasts separate from inline traces (${mobile ? "mobile" : "desktop"})`, async ({ page }) => {
    await page.setViewportSize(mobile ? { width: 390, height: 844 } : { width: 1280, height: 900 });
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    await page.evaluate(() => {
      const original = globalThis.__nyxidAssistantHttpMock;
      let failNextSend = true;
      globalThis.__nyxidAssistantHttpMock = (request) => {
        if (request.endpoint.endsWith("/turns") && failNextSend) {
          failNextSend = false;
          return new Response(JSON.stringify({ message: "private upstream diagnostic" }), {
            status: 502,
            headers: { "X-NyxID-Trace-Id": "9ad6d0fa-bb8f-489e-8fdd-f8e34960921b" },
          });
        }
        return original?.(request);
      };
    });
    await sendMessage(page, "Check my services");
    const toast = page.locator("[data-sonner-toast]").filter({ hasText: "Could not send your message" });
    await expect(toast).toBeVisible();
    await expect(toast).toContainText("NyxBot is temporarily unavailable. Try again shortly.");
    await expect(toast).not.toContainText("Trace ID:");
    await expect(toast).not.toContainText("private upstream diagnostic");
    const notice = page.getByRole("alert").filter({ hasText: "Trace ID: 9ad6d0fa-bb8f-489e-8fdd-f8e34960921b" });
    await expect(notice).toBeVisible();
    await expect(notice).toContainText("NyxBot is temporarily unavailable. Try again shortly.");
    await expect(page.locator("[data-sonner-toaster]")).toHaveAttribute("data-y-position", "top");
    const bounds = await toast.boundingBox();
    expect(bounds!.y).toBeLessThan(150);
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(mobile ? 390 : 1280);
    // A rejected send remains editable and does not silently retry itself.
    await expect(composerInput(page)).toHaveText("Check my services");
    // A successful retry clears the old inline failure.
    await sendMessage(page, "Check my services");
    await expect(page.getByText("Your connected services are ready.", { exact: false })).toBeVisible();
    await expect(notice).toHaveCount(0);
  });
}

test("an interrupted first reply keeps its trace in the adopted chat", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true, progressStallMs: 4000 } });
  await page.evaluate(() => {
    const original = globalThis.__nyxidAssistantHttpMock;
    globalThis.__nyxidAssistantHttpMock = async (request) => {
      const response = await original?.(request);
      if (response?.ok && request.endpoint.endsWith("/turns")) {
        const reader = response.body!.getReader();
        const first = await reader.read();
        await reader.cancel();
        return new Response(new ReadableStream({
          start(controller) { controller.enqueue(first.value); },
          async pull(controller) {
            await new Promise((resolve) => setTimeout(resolve, 50));
            controller.error(new TypeError("network error"));
          },
        }), { headers: { ...Object.fromEntries(response.headers), "X-NyxID-Trace-Id": "9ad6d0fa-bb8f-489e-8fdd-f8e34960921b" } });
      }
      return response;
    };
  });
  await sendMessage(page, "Check my services");
  await expect(page).toHaveURL(/c=nyxa-/);
  const toast = page.locator("[data-sonner-toast]").filter({ hasText: "Live reply interrupted" });
  await expect(toast).toBeVisible();
  await expect(toast).toContainText("NyxBot may still be working");
  await expect(toast).not.toContainText("Trace ID:");
  await expect(page.getByRole("alert").filter({ hasText: "Trace ID: 9ad6d0fa-bb8f-489e-8fdd-f8e34960921b" })).toBeVisible();
  await expect(composerInput(page)).toHaveText("");
});
