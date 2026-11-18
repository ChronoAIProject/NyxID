import { devices, expect, test, type Page, type TestInfo } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import type { KeyInfo } from "../src/types/keys";

async function openPicker(page: Page) {
  await page.getByRole("button", { name: "Add context" }).click();
  await page.getByRole("menuitem", { name: "Use a connection" }).click();
  return page.getByRole("dialog", { name: "Connection for this message" });
}

async function update(page: Page, patch: Partial<KeyInfo>) {
  await page.evaluate(async (value) => {
    const { updateConnection } =
      await import("/e2e/fixtures/composer-connections-state.ts");
    updateConnection("work", value);
  }, patch);
}

test.beforeEach(async ({ page }) => {
  await page.context().grantPermissions(["local-network-access"]);
  await page.route("**/api/**", async (route) => {
    if (new URL(route.request().url()).pathname === "/api/v1/keys") {
      const keys = await page.evaluate(
        async () =>
          (await import("/e2e/fixtures/composer-connections-state.ts"))
            .serverConnections,
      );
      await route.fulfill({ json: { keys } });
    } else await route.abort();
  });
  await page.goto("/e2e/fixtures/composer-connections.html");
  await expect(page.locator("[data-composer-textbox]")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const state =
          await import("/e2e/fixtures/composer-connections-state.ts");
        return state.client.getQueryState(state.queryKey)?.fetchStatus;
      }),
    )
    .toBe("idle");
});

test("compact composer controls share one vertical center line", async ({
  page,
}) => {
  const centers = await page
    .locator(
      "[data-composer-textbox], [data-composer-attachment], [data-composer-footer]",
    )
    .evaluateAll((elements) =>
      elements.map((element) => {
        const rect = element.getBoundingClientRect();
        return rect.top + rect.height / 2;
      }),
    );
  expect(Math.max(...centers) - Math.min(...centers)).toBeLessThan(0.5);
});

test("disabled choices expose reasons and keyboard reaches them without selecting; unverified, refresh and fallback stay selectable", async ({
  page,
}, testInfo) => {
  const dialog = await openPicker(page);
  const missing = dialog.getByRole("button", { name: /Missing credential/ });
  await expect(missing).toHaveAttribute("aria-disabled", "true");
  await expect(missing).toHaveAccessibleDescription("Credential missing");
  await missing.click({ force: true });
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: /Node without fallback/ }),
  ).toHaveAttribute("aria-disabled", "true");
  await expect(
    dialog.getByRole("button", { name: /Unavailable platform/ }),
  ).toHaveAttribute("aria-disabled", "true");
  for (const label of [
    "Unverified account",
    "Refreshable OAuth",
    "Node with fallback",
  ]) {
    await expect(
      dialog.getByRole("button", { name: new RegExp(label) }),
    ).toBeEnabled();
  }
  await page.screenshot({
    path: testInfo.outputPath("connection-availability.png"),
    fullPage: true,
    animations: "disabled",
  });
  await dialog.getByRole("textbox", { name: "Search connections" }).focus();
  await page.keyboard.press("Tab");
  await expect(missing).toBeFocused();
  await expect(missing).toHaveAccessibleName(
    "Missing credential Personal · unavailable",
  );
  await page.keyboard.press("Enter");
  await page.keyboard.press("Space");
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(
    dialog.getByRole("button", { name: /Work documents/ }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(dialog).not.toBeVisible();
  await expect(
    page.getByRole("group", { name: "Connection for this message" }),
  ).toContainText("Work documents");
});

test("a focused option becoming unavailable cannot be selected by Enter or Space", async ({
  page,
}) => {
  const dialog = await openPicker(page);
  const work = dialog.getByRole("button", { name: /Work documents/ });
  await work.focus();
  await update(page, { credential_missing: true });
  await expect(work).toHaveAttribute("aria-disabled", "true");
  await expect(work).toHaveAccessibleDescription("Credential missing");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Space");
  await expect(dialog).toBeVisible();
  await expect(
    page.getByRole("group", { name: "Connection for this message" }),
  ).toHaveCount(0);
  await dialog.getByRole("textbox", { name: "Search connections" }).focus();
  await page.keyboard.press("Tab");
  await expect(
    dialog.getByRole("button", { name: /Missing credential/ }),
  ).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(work).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(
    dialog.getByRole("button", { name: /Personal documents/ }),
  ).toBeFocused();
});

test("live selection updates its label, refuses mouse and keyboard sends on invalidation, and switches only explicitly", async ({
  page,
}, testInfo) => {
  const dialog = await openPicker(page);
  await dialog.getByRole("button", { name: /Work documents/ }).click();
  const chip = page.getByRole("group", { name: "Connection for this message" });
  await update(page, { label: "Renamed account" });
  await expect(chip).toContainText("Renamed account");
  await expect(chip).not.toContainText("Work documents");
  await update(page, { status: "refresh_failed" });
  await expect(chip.getByRole("status")).toContainText("Reconnect needed");
  const input = page.locator("[data-composer-textbox]");
  await input.fill("Keep this draft");
  await page.getByRole("button", { name: "Send message" }).click();
  await expect(input).toHaveText("Keep this draft");
  await input.press("Enter");
  await expect(input).toHaveText("Keep this draft");
  await expect(page.getByLabel("Submitted messages")).toHaveText(
    "No messages sent yet.",
  );
  await page.screenshot({
    path: testInfo.outputPath("selected-connection-unavailable.png"),
    fullPage: true,
    animations: "disabled",
  });
  await page
    .getByRole("button", { name: "Change connection for this message" })
    .click();
  await dialog.getByRole("button", { name: /Personal documents/ }).click();
  await input.press("Enter");
  await expect(page.getByLabel("Submitted messages")).toHaveText(
    "/service personal\nKeep this draft",
  );
  await expect(chip).toHaveCount(0);
});

test("selection and removal return focus to the editor; locks never reopen the picker", async ({
  page,
}) => {
  const dialog = await openPicker(page);
  await dialog.getByRole("button", { name: /Work documents/ }).click();
  const editor = page.locator("[data-composer-textbox]");
  await expect(editor).toBeFocused();
  await page
    .getByRole("button", { name: "Remove connection for this message" })
    .click();
  await expect(editor).toBeFocused();
  await openPicker(page);
  // The fixture lock is outside the modal, so invoke its real React handler.
  await page
    .getByRole("button", { name: "Toggle composer lock", includeHidden: true })
    .evaluate((button: HTMLButtonElement) => button.click());
  await expect(dialog).not.toBeVisible();
  await page.getByRole("button", { name: "Toggle composer lock" }).click();
  await expect(dialog).not.toBeVisible();
});

test.describe("coarse pointer focus", () => {
  const pixel = devices["Pixel 7"];
  test.use({
    viewport: pixel.viewport,
    screen: pixel.screen,
    userAgent: pixel.userAgent,
    deviceScaleFactor: pixel.deviceScaleFactor,
    isMobile: pixel.isMobile,
    hasTouch: pixel.hasTouch,
  });

  test("selection, close and removal return to stable controls without focusing the editor", async ({ page }) => {
    expect(await page.evaluate(() => matchMedia("(pointer: coarse)").matches)).toBe(true);
    const addContext = page.getByRole("button", { name: "Add context", exact: true });
    const change = page.getByRole("button", { name: "Change connection for this message" });
    const editor = page.locator("[data-composer-textbox]");
    const dialog = await openPicker(page);
    await dialog.getByRole("button", { name: "Close", exact: true }).click();
    await expect(addContext).toBeFocused();
    await openPicker(page);
    await page.keyboard.press("Escape");
    await expect(addContext).toBeFocused();
    await openPicker(page);
    await dialog.getByRole("button", { name: /Work documents/ }).click();
    await expect(change).toBeFocused();
    await expect(editor).not.toBeFocused();
    await openPicker(page);
    await dialog.getByRole("button", { name: "Close", exact: true }).click();
    await expect(addContext).toBeFocused();
    await change.click();
    await dialog.getByRole("button", { name: "Close", exact: true }).click();
    await expect(change).toBeFocused();
    await change.click();
    await page.keyboard.press("Escape");
    await expect(change).toBeFocused();
    await page.getByRole("button", { name: "Remove connection for this message" }).click();
    await expect(addContext).toBeFocused();
    await expect(editor).not.toBeFocused();
  });
});

test("restoring connections resets both the chip and the next picker refresh", async ({ page }) => {
  const dialog = await openPicker(page);
  await dialog.getByRole("button", { name: /Work documents/ }).click();
  const chip = page.getByRole("group", { name: "Connection for this message" });
  await page.getByRole("button", { name: "Invalidate work connection" }).click();
  await expect(chip).toContainText("Credential missing");
  await page.getByRole("button", { name: "Restore connections" }).click();
  await expect(chip).toContainText("This message");
  const refreshed = page.waitForResponse((response) => new URL(response.url()).pathname === "/api/v1/keys");
  await openPicker(page);
  const { keys } = await (await refreshed).json() as { keys: KeyInfo[] };
  expect(keys.find((key) => key.id === "work")?.credential_missing).toBeFalsy();
  await expect(dialog.getByRole("button", { name: /Work documents/ })).toHaveAttribute("aria-disabled", "false");
  await page.keyboard.press("Escape");
  await expect(chip).not.toContainText("Unavailable");
});

test("opening refreshes cached connections once, updates live choices and offers Retry on failure", async ({
  page,
}) => {
  let requests = 0;
  let fail = false;
  await page.route("**/api/v1/keys", async (route) => {
    requests += 1;
    if (fail)
      await route.fulfill({
        status: 503,
        json: { error: "unavailable", message: "Unavailable" },
      });
    else {
      await page.evaluate(async () => {
        const state =
          await import("/e2e/fixtures/composer-connections-state.ts");
        state.serverConnections[1] = {
          ...state.serverConnections[1]!,
          credential_missing: true,
        };
      });
      const keys = await page.evaluate(
        async () =>
          (await import("/e2e/fixtures/composer-connections-state.ts"))
            .serverConnections,
      );
      await route.fulfill({ json: { keys } });
    }
  });
  const dialog = await openPicker(page);
  const work = dialog.getByRole("button", { name: /Work documents/ });
  await expect(work).toHaveAccessibleDescription("Credential missing");
  expect(requests).toBe(1);
  await page.keyboard.press("Escape");
  fail = true;
  await openPicker(page);
  await expect(dialog.getByRole("alert")).toContainText(
    "Could not load connections.",
  );
  await dialog.getByRole("button", { name: "Retry" }).click();
  await expect.poll(() => requests).toBe(3);
});

test("expiry updates an open picker and chip without a cache refresh", async ({
  page,
}) => {
  const dialog = await openPicker(page);
  await dialog.getByRole("button", { name: /Work documents/ }).click();
  await page.clock.install();
  await update(page, { expires_at: new Date(Date.now() + 2000).toISOString() });
  await page
    .getByRole("button", { name: "Change connection for this message" })
    .click();
  await page.clock.fastForward(2100);
  await expect(
    dialog.getByRole("button", { name: /Work documents/ }),
  ).toHaveAccessibleDescription("Credential expired");
  await expect(
    dialog.getByRole("button", { name: /Refreshable OAuth/ }),
  ).toHaveAttribute("aria-disabled", "false");
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("group", { name: "Connection for this message" }),
  ).toContainText("Credential expired");
});

for (const variant of ["light", "dark", "mobile"] as const) {
  test(`availability evidence: ${variant}`, async ({ page }, testInfo) => {
    if (variant === "mobile")
      await page.setViewportSize({ width: 390, height: 844 });
    const mode = variant === "light" ? "light" : "dark";
    await page.evaluate(async (value) => {
      (await import("/src/stores/theme-store.ts")).useThemeStore
        .getState()
        .setMode(value);
    }, mode);
    const dialog = await openPicker(page);
    await dialog.getByRole("button", { name: /Work documents/ }).click();
    await update(page, { credential_missing: true });
    const chip = page.getByRole("group", {
      name: "Connection for this message",
    });
    await expect(chip).toContainText("Credential missing");
    await expect(
      chip.getByRole("button", {
        name: "Change connection for this message",
      }),
    ).toBeVisible();
    await expect(
      chip.getByRole("button", { name: "Remove connection for this message" }),
    ).toBeVisible();
    await expect(chip.getByRole("button", { name: "Change connection for this message" })).toHaveText("Change");
    expect(await chip.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    const evidence = process.env.NYXID_CONNECTION_EVIDENCE_DIR;
    if (evidence) await mkdir(evidence, { recursive: true });
    const shot = (name: string) =>
      evidence
        ? path.join(evidence, `phase2-${variant}-${name}.png`)
        : testInfo.outputPath(`phase2-${variant}-${name}.png`);
    await page.screenshot({
      path: shot("unavailable-chip"),
      fullPage: true,
      animations: "disabled",
    });
    await chip
      .getByRole("button", {
        name: "Change connection for this message",
      })
      .click();
    const search = dialog.getByRole("textbox", { name: "Search connections" });
    await search.focus();
    await page.keyboard.press("Tab");
    const missing = dialog.getByRole("button", { name: /Missing credential/ });
    await expect(missing).toBeFocused();
    await expect(missing).toHaveAccessibleDescription("Credential missing");
    await page.screenshot({
      path: shot("focused-picker"),
      fullPage: true,
      animations: "disabled",
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
  });
}

async function holdKeys(page: Page) {
  let release!: (patch: Partial<KeyInfo>) => void;
  let entered!: () => void;
  const started = new Promise<void>((resolve) => { entered = resolve; });
  const gate = new Promise<Partial<KeyInfo>>((resolve) => { release = resolve; });
  await page.route("**/api/v1/keys", async (route) => {
    entered();
    const patch = await gate;
    const keys = await page.evaluate(async () => (await import("/e2e/fixtures/composer-connections-state.ts")).serverConnections);
    await route.fulfill({ json: { keys: keys.map((key) => key.id === "work" ? { ...key, ...patch } : key) } });
  });
  return { started, release };
}

async function remediationEvidence(page: Page, testInfo: TestInfo, name: string) {
  const evidence = process.env.NYXID_CONNECTION_EVIDENCE_DIR;
  if (evidence) await mkdir(evidence, { recursive: true });
  await page.screenshot({
    path: evidence ? path.join(evidence, `phase6-${name}.png`) : testInfo.outputPath(`phase6-${name}.png`),
    fullPage: true,
    animations: "disabled",
  });
}

test("held refresh checks an activated option and fences selected-connection sends", async ({ page }, testInfo) => {
  const held = await holdKeys(page);
  const dialog = await openPicker(page);
  await held.started;
  const work = dialog.getByRole("button", { name: /Work documents/ });
  await work.click();
  await expect(dialog).toBeVisible();
  await expect(work).toHaveAttribute("aria-busy", "true");
  await expect(work).toHaveAccessibleDescription("Checking availability…");
  await expect(work.getByText("Checking availability…")).toHaveClass(/text-muted-foreground/);
  await expect(work.getByText("Checking availability…")).not.toHaveClass(/text-destructive/);
  await remediationEvidence(page, testInfo, "held-option");
  held.release({ credential_missing: true });
  await expect(work).toHaveAccessibleDescription("Credential missing");
  await expect(dialog).toBeVisible();
  await expect(page.getByRole("group", { name: "Connection for this message" })).toHaveCount(0);
  await remediationEvidence(page, testInfo, "settled-unavailable-option");
  await page.keyboard.press("Escape");

  const available = await holdKeys(page);
  await openPicker(page);
  await available.started;
  available.release({ credential_missing: false });
  await expect(work).toHaveAttribute("aria-disabled", "false");
  await work.click();
  await expect(dialog).not.toBeVisible();
  const refresh = await holdKeys(page);
  await openPicker(page);
  await refresh.started;
  await page.keyboard.press("Escape");
  const input = page.locator("[data-composer-textbox]");
  await input.fill("Keep until checked");
  const chip = page.getByRole("group", { name: "Connection for this message" });
  await expect(chip).not.toContainText("Unavailable");
  await expect(chip.getByRole("status")).not.toContainText("Checking");
  await expect(chip.getByText("Checking availability…")).toHaveClass(/text-muted-foreground/);
  await expect(chip.getByText("Checking availability…")).not.toHaveClass(/text-destructive/);
  await page.getByRole("button", { name: "Send message" }).click();
  await input.press("Enter");
  await expect(input).toHaveText("Keep until checked");
  await expect(page.getByRole("group", { name: "Connection for this message" })).toContainText("Checking availability…");
  await expect(chip).not.toContainText("Unavailable");
  await expect(chip.getByRole("status")).toContainText("Checking availability…");
  await expect(chip.getByRole("status")).toHaveClass(/text-muted-foreground/);
  await expect(chip.getByRole("status")).not.toHaveClass(/text-destructive/);
  await expect(page.getByLabel("Submitted messages")).toHaveText("No messages sent yet.");
  await remediationEvidence(page, testInfo, "checking-chip-retained-text");
  refresh.release({ credential_missing: false });
  await expect(page.getByRole("group", { name: "Connection for this message" })).not.toContainText("Checking availability…");
  await expect(chip.getByRole("status")).toContainText("This message");
  await input.press("Enter");
  await expect(page.getByLabel("Submitted messages")).toHaveText("/service work\nKeep until checked");
  await expect(page.getByText("Local send log · 1 messages")).toBeVisible();
});

for (const unavailable of [false, true]) {
  test(`background checking stays quiet and settles ${unavailable ? "unavailable" : "available"}`, async ({ page }, testInfo) => {
    const dialog = await openPicker(page);
    await dialog.getByRole("button", { name: /Work documents/ }).click();
    const held = await holdKeys(page);
    await openPicker(page); await held.started;
    await page.keyboard.press("Escape");
    const chip = page.getByRole("group", { name: "Connection for this message" });
    const status = chip.getByRole("status");
    await expect(chip).toContainText("Checking availability…");
    await expect(chip).not.toContainText("Unavailable");
    await expect(status).toContainText("This message");
    await expect(status).not.toContainText("Checking");
    const checking = chip.getByText("Checking availability…");
    await expect(checking).toHaveAttribute("aria-hidden", "true");
    await expect(checking).toHaveClass(/text-muted-foreground/);
    await expect(checking).not.toHaveClass(/text-destructive/);
    await expect(chip.getByRole("button", { name: "Change connection for this message" })).toBeEnabled();
    await expect(chip.getByRole("button", { name: "Remove connection for this message" })).toBeEnabled();
    await remediationEvidence(page, testInfo, `background-checking-${unavailable ? "unavailable" : "available"}`);
    held.release({ credential_missing: unavailable });
    await expect(chip).not.toContainText("Checking availability…");
    if (unavailable) {
      await expect(status).toContainText("Unavailable — choose another connection or remove · Credential missing");
      await expect(status).toHaveClass(/text-destructive/);
    } else {
      await expect(status).toContainText("This message");
      await expect(status).toHaveClass(/text-muted-foreground/);
    }
    await expect(page.getByLabel("Submitted messages")).toHaveText("No messages sent yet.");
  });
}

test("activation catches expiry when the wall clock advances without timer delivery", async ({ page }, testInfo) => {
  await page.clock.install();
  const start = await page.evaluate(() => Date.now());
  await update(page, { expires_at: new Date(start + 60000).toISOString() });
  const dialog = await openPicker(page);
  await expect.poll(() => page.evaluate(async () => {
    const state = await import("/e2e/fixtures/composer-connections-state.ts");
    return state.client.getQueryState(state.queryKey)?.fetchStatus;
  })).toBe("idle");
  const work = dialog.getByRole("button", { name: /Work documents/ });
  await expect(work).toHaveAttribute("aria-disabled", "false");
  await page.clock.setSystemTime(start + 70000);
  await work.click();
  await expect(dialog).toBeVisible();
  await expect(work).toHaveAccessibleDescription("Credential expired");
  await expect(page.getByRole("group", { name: "Connection for this message" })).toHaveCount(0);
  await remediationEvidence(page, testInfo, "activation-expired-option");
});

test("offline refresh holds activation and refuses sends until the resumed refresh settles", async ({ page, context }, testInfo) => {
  const fetchStatus = () => page.evaluate(async () => {
    const state = await import("/e2e/fixtures/composer-connections-state.ts");
    return state.client.getQueryState(state.queryKey)?.fetchStatus;
  });
  try {
    const activation = await holdKeys(page);
    await context.setOffline(true);
    const dialog = await openPicker(page);
    await expect.poll(fetchStatus).toBe("paused");
    const work = dialog.getByRole("button", { name: /Work documents/ });
    await work.click();
    await expect(dialog).toBeVisible();
    await expect(work).toHaveAttribute("aria-busy", "true");
    await expect(work).toHaveAccessibleDescription("Checking availability…");
    await expect(page.getByRole("group", { name: "Connection for this message", includeHidden: true })).toHaveCount(0);
    await remediationEvidence(page, testInfo, "offline-held-option");
    await context.setOffline(false);
    await activation.started;
    await expect.poll(fetchStatus).toBe("fetching");
    activation.release({ credential_missing: false });
    await expect.poll(fetchStatus).toBe("idle");
    await expect(dialog).not.toBeVisible();
    const chip = page.getByRole("group", { name: "Connection for this message" });
    await expect(chip).toContainText("Work documents");

    const send = await holdKeys(page);
    await context.setOffline(true);
    await openPicker(page);
    await expect.poll(fetchStatus).toBe("paused");
    await page.keyboard.press("Escape");
    const input = page.locator("[data-composer-textbox]");
    await input.fill("Keep while offline");
    await expect(chip).toContainText("Checking availability…");
    await expect(chip).not.toContainText("Unavailable");
    await expect(chip.getByRole("status")).not.toContainText("Checking");
    await page.getByRole("button", { name: "Send message" }).click();
    await input.press("Enter");
    await expect(input).toHaveText("Keep while offline");
    await expect(chip.getByRole("status")).toContainText("Checking availability…");
    await expect(chip.getByRole("status")).toHaveClass(/text-muted-foreground/);
    await expect(page.getByLabel("Submitted messages")).toHaveText("No messages sent yet.");
    await remediationEvidence(page, testInfo, "offline-checking-chip");
    await context.setOffline(false);
    await send.started;
    await expect.poll(fetchStatus).toBe("fetching");
    send.release({ credential_missing: false });
    await expect.poll(fetchStatus).toBe("idle");
    await expect(chip.getByRole("status")).toContainText("This message");
    await input.press("Enter");
    await expect(page.getByLabel("Submitted messages")).toHaveText("/service work\nKeep while offline");
    await expect(page.getByText("Local send log · 1 messages")).toBeVisible();
  } finally {
    await context.setOffline(false);
  }
});
