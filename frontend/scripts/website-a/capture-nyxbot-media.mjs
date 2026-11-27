/**
 * Capture the real development app, using its existing HTTP fixtures.
 * No DOM/UI substitutions and no provider requests. Start Vite on :2972 first.
 * Run from frontend: node scripts/website-a/capture-nyxbot-media.mjs
 * Requires Playwright Chromium and cwebp on PATH.
 */
import { chromium, expect } from "@playwright/test";
import { mkdir, rename, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { execFileSync } from "node:child_process";

const origin = process.env.NYXBOT_CAPTURE_ORIGIN ?? "http://127.0.0.1:2972";
const output = resolve("public/landing/nyx/app");
const scratch = resolve("../.playwright-mcp/app-media");
await mkdir(output, { recursive: true });
await mkdir(scratch, { recursive: true });
await mkdir(resolve(scratch, "recordings"), { recursive: true });
const browser = await chromium.launch();
const evidence = [];
const stillsOnly = process.argv.includes("--stills-only");
const mobileOnly = process.argv.includes("--mobile-only");
const task = "Check tonight’s bookings";
const result = "Sarah’s table for four is confirmed for 7:30. One vegan meal noted.";
// Change example data at the existing fixture boundary, before the app loads.
// No component, markup or behavior is replaced. The generic action card stays real.
const sampleCopy = [
  ["Ask the researcher for urgent issues", task],
  ["Found 3 urgent issues: #12, #15 and #18.", result],
  ["Tracks open GitHub issues and reports the most urgent ones.", "Looks after bookings and keeps the guest notes in order."],
  ["Find the three most urgent open GitHub issues.", "Check tonight’s table bookings and guest notes."],
  ["Find the urgent open GitHub issues again.", "Check tonight’s bookings and guest notes."],
  ["I asked the researcher to find the urgent issues.", "I asked the concierge to check tonight’s bookings."],
  ["The user wants weekly issue digests on Mondays.", "Send the booking summary at 8:00 each morning."],
  ["Label urgent issues with the p0 tag.", "Table 12 is wheelchair-accessible."],
  ["Delete agent key 'ci-bot' (nyxid_ag_12345678)", "Cancel Sarah’s 7:30 booking"],
  ["Delete agent key ci-bot", "Cancel Sarah’s 7:30 booking"],
  ["Deleted agent key ci-bot.", "Sarah’s booking is cancelled."],
  ["nyxid__delete_agent_key", "calendar__cancel_booking"],
  ["Please confirm the deletion card, then I will continue.", "Please confirm the cancellation, then I will continue."],
  ["researcher", "concierge"],
];

async function still(locator, name, options = {}) {
  const png = resolve(scratch, `${name}.png`);
  await locator.screenshot({ path: png, ...options });
  execFileSync("cwebp", ["-quiet", "-q", "86", "-m", "6", png, "-o", resolve(output, `${name}.webp`)]);
}

async function openPage(theme, mobile, record = false) {
  const viewport = mobile ? { width: 440, height: 680 } : { width: 1200, height: 640 };
  const context = await browser.newContext({
    viewport,
    colorScheme: theme,
    reducedMotion: "reduce",
    ...(record ? { recordVideo: { dir: scratch, size: viewport } } : {}),
  });
  // These navigation aids do not explain the task in a noninteractive capture.
  // Hide them before the first app paint, including in recorded frames.
  await context.route(`${origin}/assistant?*`, async route => {
    if (route.request().resourceType() !== "document") return route.fallback();
    const response = await route.fetch();
    const body = (await response.text()).replace("</head>", '<style>[role="separator"],[data-chat-index-container]{visibility:hidden!important}section[aria-label="Personal crews"]{display:none!important}</style></head>');
    await route.fulfill({ response, body });
  });
  await context.route("**/src/lib/mock-data.ts*", async route => {
    const response = await route.fetch();
    const body = (await response.text()).replaceAll("Dannick Young", "Alex Rivera").replaceAll("dannick@nyxid.dev", "alex@example.com");
    await route.fulfill({ response, body });
  });
  await context.route("**/src/lib/assistant/nyxagent-http-fixtures.ts*", async route => {
    const response = await route.fetch();
    const body = sampleCopy.reduce((text, [before, after]) => text.replaceAll(before, after), await response.text());
    await route.fulfill({ response, body });
  });
  await context.addInitScript(({ mode }) => {
    window.__nyxidAssistantHttpFaults = { nyxagentEnabled: true };
    localStorage.setItem("nyxid.theme", JSON.stringify({ state: { mode, textScale: 1.1, sidebarWidths: { dashboard: 240, assistant: 240 } }, version: 0 }));
  }, { mode: theme });
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
  await page.goto(`${origin}/assistant?mock=1&nyxbot=1`, { waitUntil: "networkidle" });
  await expect(page.getByRole("heading", { name: /^Good (morning|afternoon|evening), Alex$/ })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  return { context, page };
}

try {
  for (const theme of ["light", "dark"]) {
    // Portrait captures are actual responsive app layouts, not crops of desktop.
    for (const mobile of [false, true]) {
      if (mobileOnly && !mobile) continue;
      if (stillsOnly) continue;
      const suffix = `${theme}${mobile ? "-mobile" : ""}`;
      const { context, page } = await openPage(theme, mobile, true);
      const textbox = page.locator("[data-composer-textbox]");
      await textbox.click();
      await textbox.pressSequentially(task, { delay: 44 });
      await page.waitForTimeout(500);
      await textbox.press("Enter");
      await expect(page.getByText(`The concierge reported back: ${result}`, { exact: true })).toBeVisible({ timeout: 12000 });
      await page.waitForTimeout(800);
      await still(page, `task-${suffix}`);
      await page.waitForTimeout(2200);
      const video = page.video();
      await context.close();
      await rename(await video.path(), resolve(scratch, "recordings", `task-${suffix}.webm`));
      evidence.push({ theme, mobile, workflow: "NyxBot delegates a booking check to a specialist and receives the result", data: "Local HTTP fixtures with fictional example identity and booking data; no live provider calls" });
    }

    for (const mobile of [false, true]) {
      if (mobileOnly && !mobile) continue;
      const suffix = `${theme}${mobile ? "-mobile" : ""}`;
      const { context, page } = await openPage(theme, mobile);
      if (mobile) await page.getByRole("button", { name: "Open chats" }).click();
      await page.getByRole("button", { name: /^concierge, specialist/ }).click();
      await page.getByRole("button", { name: "Agent details", exact: true }).click();
      const dialog = page.getByRole("dialog", { name: "Agent details", exact: true });
      await dialog.getByRole("tab", { name: "Memory", exact: true }).click();
      await expect(dialog.getByText("Table 12 is wheelchair-accessible.", { exact: true })).toBeVisible();
      await page.waitForTimeout(300);
      // The real sheet fills the viewport. Capture the meaningful upper portion.
      const bounds = await dialog.boundingBox();
      await still(page, `memory-${suffix}`, { clip: { x: bounds.x, y: bounds.y, width: bounds.width, height: 570 } });
      await page.keyboard.press("Escape");
      if (mobile) await page.getByRole("button", { name: "Open chats" }).click();
      await page.getByRole("button", { name: "Home", exact: true }).click();
      await page.locator("[data-composer-textbox]").fill("Cancel Sarah’s 7:30 booking");
      await page.locator("[data-composer-textbox]").press("Enter");
      const approval = page.getByRole("region", { name: /Confirm: Cancel Sarah/ });
      await expect(approval).toBeVisible();
      await page.waitForTimeout(1000);
      await still(approval, `approval-${suffix}`);
      // Leave the request pending. Neither permission nor deletion is performed.
      await context.close();
    }
  }
  if (!mobileOnly) await writeFile(resolve(scratch, "capture-evidence.json"), JSON.stringify(evidence, null, 2));
} finally {
  await browser.close();
}
