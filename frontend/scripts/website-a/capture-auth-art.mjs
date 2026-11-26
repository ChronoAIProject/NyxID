// Captures the landing page's own closing halftone (rendered by nyx-home-motion.js) as a still for the
// sign-in panel. Text, navigation and the night sky are hidden; the ground is the panel's ground.
import { chromium } from "@playwright/test";
// Usage (with the local preview running):
// node scripts/website-a/capture-auth-art.mjs light /tmp/nyx-day.png
// cwebp -quiet -q 75 -m 6 -sharp_yuv -crop 0 0 1500 1474 /tmp/nyx-day.png -o public/landing/nyx/art/nyx-halftone-day.webp
// Repeat with dark / nyx-night.png / nyx-halftone-night.webp.
const theme = process.argv[2] ?? "light", out = process.argv[3];
if (!["light", "dark"].includes(theme) || !out) throw new Error("Usage: capture-auth-art.mjs <light|dark> <output.png>");
const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 1.5, colorScheme: theme, reducedMotion: "reduce" });
await ctx.addInitScript((t) => { try { localStorage.setItem("nyxid-theme", t); } catch {} }, theme);
const page = await ctx.newPage();
await page.goto("http://127.0.0.1:2972/", { waitUntil: "networkidle", timeout: 30000 });
await page.waitForTimeout(800);
await page.evaluate(() => document.querySelector("#end")?.scrollIntoView({ block: "end" }));
await page.waitForTimeout(3000);
await page.addStyleTag({ content: `
  .public-header,.nyx-home .nav,.nyx-home .hud,.nyx-home .close-in,.nyx-home .skip,.public-footer{visibility:hidden!important}
  .nyx-home .page-sky{display:none!important}
  .nyx-home[data-theme="dark"]{--paper:#110F18!important;background:#110F18!important}
  .nyx-home[data-theme="dark"] .close{background:#110F18!important}
  .nyx-home[data-theme="light"] .close{background:#E4E2DB!important}` });
await page.waitForTimeout(600);
const theme_ = await page.evaluate(() => document.querySelector(".nyx-home")?.dataset.theme);
await page.screenshot({ path: out, clip: { x: 0, y: 0, width: 1000, height: 1000 } });
console.log(theme, "rendered as", theme_);
await browser.close();
