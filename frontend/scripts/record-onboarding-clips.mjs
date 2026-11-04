#!/usr/bin/env node
// Records the first-run intro clips from the real assistant UI in dev mock
// mode (fixture data only, no backend, no personal data). Re-run after a UI
// change that the clips show:
//
//   npm run dev -- --port 3101 --strictPort    # in another terminal
//   node scripts/record-onboarding-clips.mjs [http://localhost:3101]
//
// Writes public/onboarding/<clip>.mp4 (web) and
// ../mobile/assets/onboarding/<clip>.gif (mobile). Needs ffmpeg on PATH.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const base = process.argv[2] ?? "http://localhost:3101";
const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const webOut = path.join(root, "public", "onboarding");
const mobileOut = path.join(root, "..", "mobile", "assets", "onboarding");
const VIEWPORT = { width: 1000, height: 560 };
const SCALE = 2;

async function say(page, text) {
  const composer = page.getByRole("textbox", { name: /Message NyxBot/ }).first();
  await composer.click();
  await composer.pressSequentially(text, { delay: 45 });
  await page.waitForTimeout(350);
  await page.keyboard.press("Enter");
}

const CLIPS = {
  // NyxBot takes a setup request end to end.
  "chief-of-staff": async (page) => {
    await say(page, "Set up a Telegram bot");
    await page.getByText(/Telegram/).last().waitFor();
    await page.waitForTimeout(3500);
  },
  // NyxBot hands work to a specialist, which reports back.
  specialists: async (page) => {
    await say(page, "Ask the researcher for urgent issues");
    await page.getByText("I asked the researcher to find the urgent issues.").waitFor();
    await page.getByText(/Found 3 urgent issues/).last().waitFor({ timeout: 20_000 });
    await page.waitForTimeout(3500);
  },
  // A destructive action waits for the owner. Allow while the reply is still
  // running, so the decision shows as a continuation line.
  control: async (page) => {
    await say(page, "Delete agent key ci-bot");
    const card = page.getByRole("region", { name: /Confirm: Delete agent key/ });
    await card.waitFor({ timeout: 20_000 });
    await page.waitForTimeout(1600);
    const allow = card.getByRole("button", { name: "Allow", exact: true });
    await allow.hover();
    await page.waitForTimeout(500);
    await allow.click();
    await page.getByText("Deleted agent key ci-bot.").last().waitFor({ timeout: 20_000 });
    await page.waitForTimeout(3000);
  },
};

/**
 * Fixture switches per clip (the same global the e2e suite uses): no partial
 * placeholder text, and continuations stored as the server stores them.
 */
const DEMO = { nyxagentEnabled: true, partialText: "", markContinuations: true };
const FAULTS = {
  "chief-of-staff": DEMO,
  specialists: DEMO,
  control: { ...DEMO, progressStallMs: 6000 },
};

fs.mkdirSync(webOut, { recursive: true });
fs.mkdirSync(mobileOut, { recursive: true });
const browser = await chromium.launch();
for (const [name, run] of Object.entries(CLIPS)) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), `onboarding-${name}-`));
  const context = await browser.newContext({
    viewport: VIEWPORT,
    deviceScaleFactor: SCALE,
    colorScheme: "dark",
  });
  const page = await context.newPage();
  if (FAULTS[name]) {
    await page.addInitScript((faults) => {
      window.__nyxidAssistantHttpFaults = faults;
    }, FAULTS[name]);
  }
  await page.goto(`${base}/assistant?mock=1&nyxbot=1`);
  await page.getByRole("textbox", { name: /Message NyxBot/ }).first().waitFor({ timeout: 30_000 });
  await page.waitForTimeout(800);
  // Only the chat slab: the transcript and composer, not the sidebar.
  const slab = await page.locator("main").first().boundingBox();
  // Playwright's video recorder and Chrome's headless screencast are 1x, so
  // capture 2x screenshots of the slab in a loop. Frames are timestamped, so
  // pacing stays real.
  const frames = [];
  let recording = true;
  const capture = (async () => {
    while (recording) {
      const file = path.join(dir, `${String(frames.length).padStart(5, "0")}.png`);
      await page.screenshot({ path: file, clip: slab, animations: "allow", caret: "initial" });
      frames.push({ file, at: Date.now() / 1000 });
    }
  })();
  await run(page);
  recording = false;
  await capture;
  await context.close();
  const list = frames
    .map((frame, index) => {
      const next = frames[index + 1]?.at ?? frame.at + 1;
      return `file '${frame.file}'\nduration ${Math.max(next - frame.at, 1 / 60).toFixed(4)}`;
    })
    .join("\n");
  const concat = path.join(dir, "frames.txt");
  fs.writeFileSync(concat, `${list}\nfile '${frames.at(-1).file}'\n`);
  const mp4 = path.join(webOut, `${name}.mp4`);
  execFileSync("ffmpeg", [
    "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", concat,
    "-vf", "fps=30,scale=1280:-2:flags=lanczos", "-c:v", "libx264", "-preset", "slow",
    "-crf", "24", "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-an", mp4,
  ]);
  execFileSync("ffmpeg", [
    "-y", "-loglevel", "error", "-i", mp4, "-vf",
    "fps=12,scale=600:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96[p];[b][p]paletteuse=dither=bayer:bayer_scale=4",
    path.join(mobileOut, `${name}.gif`),
  ]);
  fs.rmSync(dir, { recursive: true, force: true });
  console.log(`${name}: ${(fs.statSync(mp4).size / 1024).toFixed(0)} KB mp4, ${(fs.statSync(path.join(mobileOut, `${name}.gif`)).size / 1024).toFixed(0)} KB gif`);
}
await browser.close();
