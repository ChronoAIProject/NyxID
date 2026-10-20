import { expect, test } from "@playwright/test";
import { openAssistant, composerInput } from "./helpers";
import { installVoiceFixture } from "./voice-fixture";
// Virtual hardware, but native Chromium permissions and getUserMedia in the
// permission recovery cases. Other cases explicitly install scripted media.
test.use({
  channel: "chromium",
  launchOptions: { args: ["--use-fake-device-for-media-stream"] },
});
const faults = {
  nyxagentEnabled: true,
  voiceEnabled: true,
  modelControlsEnabled: true,
};

for (const mode of ["dictation", "voice"] as const)
  test(`${mode} requests permission on first activation before any metered connection`, async ({
    page,
  }) => {
    const calls = await installVoiceFixture(page, "pending");
    if (mode === "dictation")
      await page.route(
        "**/api/v1/assistant/nyxagent/dictation/options?*",
        (route) =>
          route.fulfill({
            status: 404,
            json: { error: "not_found", error_code: 404, message: "Not found" },
          }),
      );
    await openAssistant(page, { faults });
    const draft = mode === "dictation" ? "Keep my draft" : "";
    await composerInput(page).fill(draft);
    const action = page.getByRole("button", {
      name: mode === "dictation" ? "Dictate a message" : "Open voice call",
    });
    // Hovering the main icon must leave settings closed and capture untouched.
    await action.hover();
    await expect(page.getByRole("switch", { name: "Hold to record" })).toHaveCount(0);
    expect(
      await page.evaluate(
        () =>
          (
            window as unknown as {
              __composerVoiceFixture: { requests: number };
            }
          ).__composerVoiceFixture.requests,
      ),
    ).toBe(0);
    await action.click();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (
              window as unknown as {
                __composerVoiceFixture: { requests: number };
              }
            ).__composerVoiceFixture.requests,
        ),
      )
      .toBe(1);
    await expect(page.getByText(/Allow microphone access/)).toBeVisible();
    expect(calls.starts).toBe(0);
    expect(calls.dictations).toBe(0);
    await page.evaluate(() =>
      (
        window as unknown as {
          __composerVoiceFixture: { grant: () => void };
        }
      ).__composerVoiceFixture.grant(),
    );
    if (mode === "dictation") {
      await expect(page.getByRole("main").getByRole("alert")).toContainText(
        "Dictation is unavailable on the connected server",
      );
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (
                window as unknown as {
                  __composerVoiceFixture: { stopped: number };
                }
              ).__composerVoiceFixture.stopped,
          ),
        )
        .toBe(1);
      expect(calls.dictations).toBe(0);
    } else {
      await expect(
        page.getByRole("button", { name: "End call", exact: true }),
      ).toBeVisible();
      expect(calls.starts).toBe(1);
      await page.getByRole("button", { name: "End call", exact: true }).click();
    }
    await expect(composerInput(page)).toHaveText(draft);
  });

test("voice credit rejection releases capture and an explicit retry starts a fresh request", async ({
  page,
}) => {
  const calls = await installVoiceFixture(page);
  const attempts: string[] = [];
  await page.route("**/voice-sessions", (route) => {
    attempts.push(
      (route.request().postDataJSON() as { client_request_id: string })
        .client_request_id,
    );
    if (attempts.length === 1)
      return route.fulfill({
        status: 402,
        json: {
          error: "insufficient_credits",
          error_code: 1300,
          message: "Insufficient voice credits",
          details: {
            stage: "billing_reservation",
            reason: "billing_reservation",
          },
        },
      });
    return route.fallback();
  });
  await openAssistant(page, { faults });
  await page.getByRole("button", { name: "Dictate a message" }).hover();
  await page.getByRole("button", { name: "Configure dictation" }).hover();
  await page.getByRole("button", { name: "MacBook Air Microphone" }).click();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Open voice call" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Insufficient voice credits",
  );
  await expect(page.locator(".composer-audio-surface")).not.toHaveAttribute(
    "data-audio-active",
  );
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as unknown as {
              __composerVoiceFixture: { stopped: number };
            }
          ).__composerVoiceFixture.stopped,
      ),
    )
    .toBe(2);
  expect(attempts).toHaveLength(1);
  expect(calls.starts).toBe(0);
  await page.getByRole("button", { name: "Retry voice", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "End call", exact: true }),
  ).toBeVisible();
  expect(attempts).toHaveLength(2);
  expect(new Set(attempts).size).toBe(2);
  expect(calls.starts).toBe(1);
  expect(
    await page.evaluate(
      () =>
        (
          window as unknown as {
            __composerVoiceFixture: { captureDevices: (string | null)[] };
          }
        ).__composerVoiceFixture.captureDevices,
    ),
  ).toEqual(["built-in", "built-in", "built-in"]);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.getByRole("button", { name: "End call", exact: true }).click();
});

test.describe("browser microphone permission recovery", () => {
  for (const mode of ["dictation", "voice"] as const)
    test(`${mode} respects browser denial and uses a newly granted permission`, async ({
      page,
      context,
      baseURL,
    }) => {
      const calls = await installVoiceFixture(page);
      await page.addInitScript(() => {
        // Remove the scripted capture fixture to exercise the browser API itself.
        Reflect.deleteProperty(navigator, "mediaDevices");
      });
      if (mode === "dictation")
        await page.route(
          "**/api/v1/assistant/nyxagent/dictation/options?*",
          (route) =>
            route.fulfill({
              status: 404,
              json: {
                error: "not_found",
                error_code: 404,
                message: "Not found",
              },
            }),
        );
      await context.grantPermissions([], { origin: baseURL });
      await openAssistant(page, { faults });
      expect(
        await page.evaluate(
          async () =>
            (
              await navigator.permissions.query({
                name: "microphone" as PermissionName,
              })
            ).state,
        ),
      ).toBe("denied");
      const action = page.getByRole("button", {
        name: mode === "dictation" ? "Dictate a message" : "Open voice call",
      });
      await action.click();
      await expect(
        page
          .getByRole("alert")
          .filter({ hasText: "Microphone access was denied" })
          .first(),
      ).toBeVisible();
      expect(calls.starts).toBe(0);
      expect(calls.dictations).toBe(0);
      await context.grantPermissions(["microphone"], { origin: baseURL });
      expect(
        await page.evaluate(
          async () =>
            (
              await navigator.permissions.query({
                name: "microphone" as PermissionName,
              })
            ).state,
        ),
      ).toBe("granted");
      await action.click();
      if (mode === "dictation") {
        await expect(
          page
            .getByRole("alert")
            .filter({
              hasText: "Dictation is unavailable on the connected server",
            })
            .first(),
        ).toBeVisible();
        expect(calls.dictations).toBe(0);
      } else {
        await expect(
          page.getByRole("button", { name: "End call", exact: true }),
        ).toBeVisible();
        expect(calls.starts).toBe(1);
        await page
          .getByRole("button", { name: "End call", exact: true })
          .click();
      }
    });
});
for (const width of [320, 390, 1280])
  test(`all composer controls fit at ${width}px with a usable editor`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await installVoiceFixture(page);
    await openAssistant(page, { faults });
    const send = page.getByRole("button", { name: "Send message", exact: true });
    const chevron = page.getByRole("button", { name: "Configure dictation" });
    await expect(chevron).toHaveCSS("opacity", "0");
    await expect(chevron).toHaveCSS("width", "0px");
    await expect(page.locator(".dictation-split-control")).toHaveCSS("width", "32px");
    await expect(send).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Open voice call" })).toBeVisible();
    await composerInput(page).fill(" \n ");
    await expect(send).toHaveCount(0);
    await composerInput(page).fill("A draft");
    await expect(send).toBeEnabled();
    await expect(page.getByRole("button", { name: "Open voice call" })).toHaveCount(0);
    const microphone = page.getByRole("button", { name: "Dictate a message" });
    await microphone.hover();
    await expect(page.getByRole("switch", { name: "Hold to record" })).toHaveCount(0);
    await expect(chevron).toHaveCSS("width", "20px");
    await expect(chevron).toHaveCSS("opacity", "1");
    const composer = await page.locator("[data-composer-input]").boundingBox();
    expect(composer).not.toBeNull();
    const controls = await page
      .locator("[data-composer-input] button:visible")
      .all();
    const boxes = [];
    for (const control of controls) {
      const b = (await control.boundingBox())!;
      expect(b.x).toBeGreaterThanOrEqual(composer!.x);
      expect(b.x + b.width).toBeLessThanOrEqual(
        Math.min(width, composer!.x + composer!.width),
      );
      boxes.push(b);
    }
    for (let a = 0; a < boxes.length; a++)
      for (let b = a + 1; b < boxes.length; b++) {
        const x = boxes[a]!,
          y = boxes[b]!;
        expect(
          x.x + x.width <= y.x + 0.5 ||
            y.x + y.width <= x.x + 0.5 ||
            x.y + x.height <= y.y + 0.5 ||
            y.y + y.height <= x.y + 0.5,
        ).toBe(true);
      }
    expect((await composerInput(page).boundingBox())!.width).toBeGreaterThan(
      120,
    );
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await chevron.hover();
    await expect(page.getByRole("switch", { name: "Hold to record" })).toBeVisible();
    await expect(page.getByText("Credit usage", { exact: true })).toBeVisible();
    const menu = await page.getByRole("dialog").boundingBox();
    expect(menu!.x).toBeGreaterThanOrEqual(0);
    expect(menu!.x + menu!.width).toBeLessThanOrEqual(width);
    await page.screenshot({ path: testInfo.outputPath("dictation-settings.png"), animations: "disabled" });
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toBeHidden();
    await composerInput(page).fill("");
    await expect(send).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Open voice call" })).toBeVisible();
    await page.mouse.move(0, 0);
    await expect(chevron).toHaveCSS("opacity", "0");
    await expect(chevron).toHaveCSS("width", "0px");
    await page.locator("[data-composer-input]").screenshot({ path: testInfo.outputPath("empty-composer.png"), animations: "disabled" });
  });
test("dictation preserves rich selected draft and attachments; confirm focuses editor and one undo restores", async ({
  page,
}) => {
  const calls = await installVoiceFixture(page);
  await openAssistant(page, { faults });
  await page.getByLabel("Choose attachments").setInputFiles({
    name: "notes.md",
    mimeType: "text/markdown",
    buffer: Buffer.from("notes"),
  });
  await expect(
    page
      .getByRole("list", { name: "Attachments", exact: true })
      .getByRole("status"),
  ).toHaveText("Ready");
  const input = composerInput(page);
  await input.fill("Review @fi");
  await page.getByRole("option", { name: /file-1/ }).click();
  await input.press("End");
  await input.press("ControlOrMeta+ArrowRight");
  await input.press("End");
  await page.keyboard.type("old world");
  await input.evaluate((element) => {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    let node: Node | null;
    while ((node = walker.nextNode())) {
      const i = node.textContent?.indexOf("old") ?? -1;
      if (i >= 0) {
        const range = document.createRange();
        range.setStart(node, i);
        range.setEnd(node, i + 3);
        const s = window.getSelection()!;
        s.removeAllRanges();
        s.addRange(range);
        element.dispatchEvent(new Event("selectionchange", { bubbles: true }));
        break;
      }
    }
  });
  await page.getByRole("button", { name: "Dictate a message" }).click();
  await expect(page.getByLabel("Dictation review")).toContainText(
    "first second interim",
  );
  await expect(page.getByText(/credits per input audio second/)).toHaveCount(0);
  await page.getByRole("button", { name: "Stop dictation for review" }).click();
  await expect(page.locator(".composer-audio-surface")).not.toHaveAttribute(
    "data-audio-active",
  );
  await page.getByRole("button", { name: "Insert dictation" }).click();
  await expect(input).toBeFocused();
  await expect(input).toContainText("first second world");
  await expect(input.locator('[data-mention-handle="file-1"]')).toBeVisible();
  await expect(
    page.getByRole("list", { name: "Attachments", exact: true }),
  ).toBeVisible();
  await input.press("ControlOrMeta+z");
  await expect(input).toContainText("old world");
  expect(calls.dictations).toBe(1);
  expect(calls.starts).toBe(0);
  await page.getByRole("button", { name: "Dictate a message" }).hover();
  await page.getByRole("button", { name: "Configure dictation" }).hover();
  await expect(page.getByText(/credits per input audio second/)).toBeHidden();
  await page.getByText("Credit usage", { exact: true }).click();
  await expect(page.getByText(/credits per input audio second/)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByText(/credits per input audio second/)).toHaveCount(0);
});
test("permission cancellation fences late grants and escape restores selection", async ({
  page,
}) => {
  const calls = await installVoiceFixture(page, "pending");
  await openAssistant(page, { faults });
  const input = composerInput(page);
  await input.fill("Keep this draft");
  await input.press("ControlOrMeta+a");
  await page.getByRole("button", { name: "Dictate a message" }).click();
  await expect(
    page.getByText("Allow microphone access in your browser…"),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(input).toBeFocused();
  await expect(input).toHaveText("Keep this draft");
  await page.evaluate(() => {
    (
      window as unknown as { __composerVoiceFixture: { grant: () => void } }
    ).__composerVoiceFixture.grant();
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { __composerVoiceFixture: { stopped: number } })
            .__composerVoiceFixture.stopped,
      ),
    )
    .toBe(1);
  expect(calls.dictations).toBe(0);
});
test("integrated active voice uses real fixture levels, keeps one session and provider fragment delivery", async ({
  page,
}) => {
  const calls = await installVoiceFixture(page);
  await openAssistant(page, { faults });
  await page.getByRole("button", { name: "Open voice call" }).click();

  await expect(
    page.getByRole("button", { name: "End call", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("A real event-shaped live caption"),
  ).toBeVisible();
  await expect(page.getByText(/credits per second/)).toHaveCount(0);
  await page.getByRole("button", { name: "Voice settings", exact: true }).click();
  await expect(page.getByText(/credits per second/)).toBeHidden();
  await page.getByText("Credit usage", { exact: true }).click();
  await expect(page.getByText(/credits per second/)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByText(/credits per second/)).toHaveCount(0);
  await page.getByRole("button", { name: "Open voice call" }).click();
  const waveform = page.getByRole("button", { name: "Open voice call" });
  const primary = await waveform.evaluate((element) => {
    const probe = document.createElement("span");
    probe.style.color = "var(--color-primary)";
    element.append(probe);
    const value = getComputedStyle(probe).color;
    probe.remove();
    return value;
  });
  await waveform.hover();
  await expect(waveform).toHaveCSS("color", primary);
  await waveform.focus();
  await expect(waveform).toHaveCSS("color", primary);
  await expect(
    page
      .locator(".composer-audio-surface")
      .getByRole("region", { name: "Voice conversation" }),
  ).toBeVisible();
  expect(calls.starts).toBe(1);
  expect(calls.ends).toBe(0);
  await expect
    .poll(() =>
      page
        .locator(".composer-audio-surface")
        .evaluate((element) =>
          Number(
            getComputedStyle(element).getPropertyValue(
              "--composer-audio-level",
            ),
          ),
        ),
    )
    .toBeGreaterThan(0);
  // Check input separately: output amplitude cannot prove automatic capture.
  await expect
    .poll(async () =>
      Number(
        await page
          .locator(".composer-audio-surface")
          .getAttribute("data-audio-input"),
      ),
    )
    .toBeGreaterThan(0);
  expect(
    await page
      .locator(".composer-audio-surface")
      .evaluate(
        (element) => getComputedStyle(element, "::before").backgroundImage,
      ),
  ).not.toBe("none");
  await expect(page.locator('[data-voice-delivery="played"]')).toContainText(
    "Played.",
  );
  await expect(page.locator('[data-voice-delivery="pending"]')).toContainText(
    "Pending.",
  );
  await page.getByRole("button", { name: "End call", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Open voice call" }),
  ).toHaveAttribute("aria-pressed", "false");
  await expect.poll(() => calls.ends).toBe(1);
});

test("microphone settings are dynamic and read-only on hover, including when server dictation is unavailable", async ({
  page,
}) => {
  const calls = await installVoiceFixture(page);
  await page.route(
    "**/api/v1/assistant/nyxagent/dictation/options?*",
    (route) =>
      route.fulfill({
        status: 404,
        json: { error: "not_found", error_code: 404, message: "Not found" },
      }),
  );
  await openAssistant(page, { faults });
  const microphone = page.getByRole("button", { name: "Dictate a message" });
  await microphone.hover();
  const toggle = page.getByRole("switch", { name: "Hold to record" });
  await expect(toggle).toHaveCount(0);
  await microphone.focus();
  await expect(toggle).toHaveCount(0);
  const chevron = page.getByRole("button", { name: "Configure dictation" });
  await chevron.hover();
  await expect(toggle).toBeVisible();
  const micBox = (await microphone.boundingBox())!;
  const chevronBox = (await chevron.boundingBox())!;
  expect(chevronBox.x).toBeGreaterThanOrEqual(micBox.x + micBox.width);
  // Measure after the popup's entrance scale settles.
  await expect.poll(async () => {
    const menuBox = (await page.getByRole("dialog").boundingBox())!;
    const triggerBox = (await chevron.boundingBox())!;
    return Math.abs(menuBox.x + menuBox.width - triggerBox.x - triggerBox.width);
  }).toBeLessThan(1);
  await expect(microphone.locator("svg")).toHaveCSS("width", "20px");
  await expect(page.getByRole("button", { name: "Open voice call" }).locator("svg")).toHaveCSS("width", "20px");
  await expect(
    page.getByRole("button", { name: "MacBook Air Microphone" }),
  ).toBeVisible();
  // Cross the portaled popup gap, then leave both regions without clicking.
  await page.getByRole("dialog").hover();
  await expect(toggle).toBeVisible();
  await page.mouse.move(10, 10);
  await expect(page.getByRole("dialog")).toBeHidden();
  await microphone.hover();
  await chevron.hover();
  await expect(toggle).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __composerVoiceFixture: { requests: number } })
          .__composerVoiceFixture.requests,
    ),
  ).toBe(0);
  await toggle.click();
  await expect(toggle).toBeChecked();
  await page.getByRole("button", { name: "Test microphone" }).click();
  await expect
    .poll(() =>
      page
        .getByLabel("Microphone input level")
        .locator("[style]")
        .evaluate((el) => getComputedStyle(el).transform),
    )
    .not.toBe("matrix(0, 0, 0, 1, 0, 0)");
  await page.mouse.move(10, 10);
  await expect(page.getByRole("dialog")).toBeHidden();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { __composerVoiceFixture: { stopped: number } })
            .__composerVoiceFixture.stopped,
      ),
    )
    .toBe(1);
  expect(calls.starts).toBe(0);
  expect(calls.dictations).toBe(0);
  await chevron.focus();
  await page.keyboard.press("Enter");
  await expect(toggle).toBeVisible();
  await toggle.focus();
  await page.keyboard.press("Space");
  await expect(toggle).not.toBeChecked();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toBeHidden();
});
