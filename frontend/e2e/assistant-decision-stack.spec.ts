import { expect, test } from "@playwright/test";
import { composerInput, openAssistant, sendMessage, stopButton } from "./helpers";

for (const width of [390, 1440]) {
  test(`pending decisions stay stacked above the composer at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 844 });
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    await page.evaluate(() => {
      const original = globalThis.__nyxidAssistantHttpMock!;
      const created = new Date(Date.now() - 1000).toISOString();
      const expires = new Date(Date.now() + 600_000).toISOString();
      const acknowledgements = [
        { id: "11111111-1111-4111-8111-111111111111", summary: "Create a test spreadsheet", authored_skill: null },
        { id: "22222222-2222-4222-8222-222222222222", summary: "Review the test skill", authored_skill: { agent_id: "test-agent", proposal_id: "test-proposal", revision: 0, skills_revision: 0 } },
      ].map((row) => ({
        ...row, kind: "action", status: "pending", decider: "user", decided_by: null as string | null,
        reason: null, service_slug: null, service_name: null, tool_name: "test_action",
        created_at: created, decided_at: null as string | null, expires_at: expires,
      }));
      const proxy = { id: "proxy-review", status: "pending", service_slug: "google-sheets", service_name: "Google Sheets", summary: "POST /v4/spreadsheets", approval_mode: "per_request", agent_key_prefix: "nyxid_ag_test", created_at: created, expires_at: expires };
      (window as unknown as { settleStackProxy: () => void }).settleStackProxy = () => { proxy.status = "rejected"; };
      globalThis.__nyxidAssistantHttpMock = async (request) => {
        if (request.endpoint.includes("/learning/proposals/test-proposal")) return Response.json({
          id: "test-proposal", agent_id: "test-agent", agent_name: "NyxBot", revision: 0, skills_revision: 0, current_skills_revision: 0,
          status: acknowledgements[1]!.status === "pending" ? "pending" : "rejected",
          state: acknowledgements[1]!.status === "pending" ? "active" : "discarded",
          actions: acknowledgements[1]!.status === "pending" ? ["deny", "publish"] : [], name: "hello-world", version: "1.0",
          files: [{ path: "SKILL.md", content: "# Hello world\n\nWrite a friendly greeting.\n\n" + Array.from({ length: 35 }, (_, i) => `Review step ${i + 1}`).join("\n") }],
        });
        if (request.init.method === "POST" && request.endpoint.includes("/acknowledgements/")) {
          const row = acknowledgements.find((item) => request.endpoint.endsWith(item.id));
          if (row) {
            row.status = "denied"; row.decided_by = "user"; row.decided_at = new Date().toISOString();
            return Response.json(row);
          }
        }
        const response = await original(request);
        if (response?.ok && request.init.method === "GET" && /^\/assistant\/nyxagent\/conversations\/[^/?]+(?:\?|$)/.test(request.endpoint)) {
          const body = await response.json();
          const turn = body.messages?.at(-1)?.turn_id;
          body.acknowledgements = acknowledgements.map((row) => ({ ...row, creation_turn_id: turn }));
          body.approvals = proxy.status === "pending" ? [proxy] : [];
          body.approval_history = [proxy];
          return Response.json(body);
        }
        return response;
      };
    });
    await sendMessage(page, "List connected services");
    await expect(stopButton(page)).not.toBeVisible();
    const stack = page.getByRole("region", { name: "3 requests need your decision" });
    await expect(stack).toBeVisible();
    await expect(stack.getByText("1 out of 3")).toBeVisible();
    await expect(stack.getByRole("button", { name: "Allow", exact: true })).toBeVisible();
    await expect(page.locator("[data-chat-transcript]").getByRole("button", { name: "Allow", exact: true })).toHaveCount(0);
    await expect(page.locator("[data-chat-transcript]").getByRole("button", { name: "Review decision: Create a test spreadsheet" })).toBeVisible();
    expect((await stack.boundingBox())!.y + (await stack.boundingBox())!.height).toBeLessThanOrEqual((await composerInput(page).boundingBox())!.y);
    await expect(composerInput(page)).toBeInViewport();
    await expect(stack.locator("[data-decision-stack-layer]")).toHaveCount(2);

    await page.locator("[data-chat-transcript]").evaluate((element) => { element.scrollTop = 0; element.dispatchEvent(new Event("scroll")); });
    await expect(stack).toBeVisible();
    await stack.getByRole("button", { name: "Next decision" }).click();
    await expect(stack.getByRole("button", { name: "Review full draft" })).toBeInViewport({ ratio: 1 });
    await expect(stack.getByRole("button", { name: "Allow and publish" })).toHaveCount(0);
    await expect(stack.getByText("2 out of 3")).toBeVisible();
    await expect(stack.getByText("1 file · 39 lines", { exact: true })).toBeVisible();
    await expect(composerInput(page)).toBeInViewport();
    const imagePath = testInfo.outputPath(`decision-stack-${width}.png`);
    await page.screenshot({ path: imagePath, animations: "disabled" });
    await testInfo.attach("pending-decision-stack", { path: imagePath, contentType: "image/png" });
    expect((await stack.boundingBox())!.height).toBeLessThan(330);
    await stack.getByRole("button", { name: "Review full draft" }).click();
    const review = page.getByRole("dialog", { name: "Review skill draft: hello-world · 1.0" });
    const allow = review.getByRole("button", { name: "Allow and publish" });
    await expect(allow).toBeInViewport({ ratio: 1 });
    await expect(review.getByText("Publishing also requires your account's Ornn create or update permission. Approving this draft does not change your Ornn role.")).toBeInViewport({ ratio: 1 });
    const initialReviewImage = testInfo.outputPath(`decision-review-open-${width}.png`);
    await page.screenshot({ path: initialReviewImage, animations: "disabled" });
    await testInfo.attach("full-decision-review-open", { path: initialReviewImage, contentType: "image/png" });
    await review.locator("[data-decision-review-body]").evaluate((element) => { element.scrollTop = element.scrollHeight; });
    await expect(review.getByText(/Review step 35/)).toBeVisible();
    await expect(allow).toBeInViewport({ ratio: 1 });
    const fullReviewImage = testInfo.outputPath(`decision-review-${width}.png`);
    await page.screenshot({ path: fullReviewImage, animations: "disabled" });
    await testInfo.attach("full-decision-review", { path: fullReviewImage, contentType: "image/png" });
    await review.getByRole("button", { name: "Close", exact: true }).click();
    await expect(stack.getByRole("button", { name: "Review full draft" })).toBeFocused();
    const settings = page.locator("[data-composer-settings-trigger]");
    const settingsLayout = await settings.evaluate((element) => {
      const text = element.querySelector("span")!.getBoundingClientRect();
      const icon = element.querySelector("svg")!.getBoundingClientRect();
      return Math.abs((text.top + text.height / 2) - (icon.top + icon.height / 2));
    });
    expect(settingsLayout).toBeLessThan(2);
    const settingsLabel = settings.locator("span").first();
    await expect(settingsLabel).toHaveText("Auto");
    expect(await settingsLabel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    if (width === 390) {
      // A short visual viewport approximates the space left by a phone keyboard.
      await page.setViewportSize({ width, height: 480 });
      await expect(composerInput(page)).toBeInViewport();
      await expect(stack.getByRole("button", { name: "Collapse decisions" })).toBeInViewport();
      await expect.poll(async () => (await stack.boundingBox())!.y).toBeGreaterThanOrEqual(0);
      await page.setViewportSize({ width, height: 844 });
    }

    await stack.getByRole("button", { name: "Previous decision" }).click();
    await stack.getByRole("button", { name: "Deny", exact: true }).click();
    const remaining = page.getByRole("region", { name: "2 requests need your decision" });
    await expect(remaining).toBeVisible();
    await expect(remaining.getByText("1 out of 2")).toBeVisible();
    await expect(remaining.getByRole("button", { name: "Review full draft" })).toBeDisabled();
    await expect(remaining.getByRole("button", { name: "Review full draft" })).toBeEnabled();

    await page.evaluate(() => (window as unknown as { settleStackProxy: () => void }).settleStackProxy());
    await expect(page.getByRole("region", { name: "Needs your decision", exact: true })).toBeVisible({ timeout: 15_000 });
    await expect(page.locator("[data-chat-transcript]").getByText("Request denied", { exact: true })).toBeAttached();
    const last = page.getByRole("region", { name: "Needs your decision", exact: true });
    await last.getByRole("button", { name: "Collapse decisions" }).click();
    await expect(composerInput(page)).toBeInViewport();
    await last.getByRole("button", { name: "Expand decisions" }).click();
    await page.emulateMedia({ reducedMotion: "reduce" });
    await last.getByRole("button", { name: "Review full draft" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Deny", exact: true }).click();
    await expect(page.locator("[data-decision-stack]")).toHaveCount(0);
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(composerInput(page)).toBeFocused();
  });
}
