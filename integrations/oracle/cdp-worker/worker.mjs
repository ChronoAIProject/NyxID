#!/usr/bin/env node
// NyxID Oracle CDP worker.
//
// A lower-friction alternative to the Tampermonkey userscript: instead of
// installing a userscript and babysitting a tab, this attaches to your
// already-running, already-logged-in Chrome over the DevTools Protocol and
// drives the ChatGPT tab for you. Same NyxID worker API, same proven answer
// extraction — but no extension to install and it runs as a background daemon.
//
// Because it drives your REAL Chrome (real session, real TLS fingerprint, the
// Cloudflare clearance you already earned by logging in normally), it is far
// less bot-detectable than a fresh headless browser.
//
// Setup (two commands — see README.md):
//   1. Launch Chrome with a debug port (and your normal profile, logged into
//      ChatGPT):
//        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
//          --remote-debugging-port=9222 --user-data-dir="$HOME/.nyxid-chrome"
//   2. Run this worker:
//        NYXID_BASE_URL=https://auth.nyxid.dev \
//        NYXID_WORKER_TOKEN=nyx_owk_... \
//        node worker.mjs
//
// Requires: Node 18+ (built-in fetch) and `npm i` (playwright-core only).

import { chromium } from "playwright-core";
import { lookup } from "node:dns/promises";
import { isIP } from "node:net";
import { readFileSync } from "node:fs";

const BASE_URL = (process.env.NYXID_BASE_URL || "").replace(/\/$/, "");
// Prefer a token file (NYXID_WORKER_TOKEN_FILE) so the long-lived worker token
// stays out of shell history and the process environment (`ps e`,
// /proc/<pid>/environ). Falls back to NYXID_WORKER_TOKEN for convenience.
const TOKEN = (() => {
  const file = process.env.NYXID_WORKER_TOKEN_FILE;
  if (file) return readFileSync(file, "utf8").trim();
  return process.env.NYXID_WORKER_TOKEN || "";
})();
const LABEL = process.env.NYXID_WORKER_LABEL || "tab_1";
const CDP_URL = process.env.CHROME_CDP_URL || "http://localhost:9222";
const SCRIPT_VERSION = "cdp-1.3-image";
const POLL_MS = Number(process.env.NYXID_POLL_MS || 5000);
const STABLE_INTERVAL_MS = 8000;
const MAX_WAIT_MS = Number(process.env.NYXID_MAX_WAIT_MS || 2 * 60 * 60 * 1000); // 2h
// Wedge guard: if ChatGPT has clearly stopped (not generating) yet produced
// nothing extractable after this long, fail the task fast and free the slot
// instead of spinning to MAX_WAIT_MS. Mirrors the userscript's
// NO_OUTPUT_IDLE_TIMEOUT (420s).
const NO_OUTPUT_IDLE_MS = Number(process.env.NYXID_NO_OUTPUT_IDLE_MS || 7 * 60 * 1000);
const HEARTBEAT_MS = 60000;
// Result-image caps (the server re-validates and caps lower-or-equal). Kept
// below the 16 MiB worker body cap once base64-inflated (~33%).
const MAX_IMAGES = Number(process.env.NYXID_MAX_IMAGES || 4);
const MAX_IMAGE_BYTES = Number(process.env.NYXID_MAX_IMAGE_BYTES || 6 * 1024 * 1024);
const MAX_IMAGES_TOTAL_BYTES = Number(process.env.NYXID_MAX_IMAGES_TOTAL_BYTES || 9 * 1024 * 1024);

if (!BASE_URL || !TOKEN) {
  console.error(
    "Missing config. Set NYXID_BASE_URL and the pool worker token (nyx_owk_...) " +
      "via NYXID_WORKER_TOKEN_FILE (preferred) or NYXID_WORKER_TOKEN."
  );
  process.exit(1);
}

// Refuse to put the long-lived worker token on the wire in cleartext. Allow
// https:// anywhere; allow http:// only for loopback (local dev). This is the
// one credential-to-network risk that isn't intrinsic to the relay design.
{
  let u;
  try {
    u = new URL(BASE_URL);
  } catch {
    console.error(`Invalid NYXID_BASE_URL: ${BASE_URL}`);
    process.exit(1);
  }
  const loopback = ["localhost", "127.0.0.1", "::1", "[::1]"].includes(u.hostname);
  if (u.protocol !== "https:" && !(u.protocol === "http:" && loopback)) {
    console.error(
      `Refusing to send the worker token over ${u.protocol}//${u.hostname} — ` +
        "use https:// (http:// is allowed only for localhost)."
    );
    process.exit(1);
  }
}

const API = `${BASE_URL}/api/v1/oracle/worker`;

function log(msg) {
  console.log(`[nyxid-cdp ${new Date().toISOString()}] ${msg}`);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ── NyxID worker API (Bearer worker token) ───────────────────────────────
function httpError(method, path, status) {
  const err = new Error(`${method} ${path} → ${status}`);
  err.status = status;
  return err;
}
async function apiGet(path) {
  const res = await fetch(`${API}${path}`, {
    headers: { Authorization: `Bearer ${TOKEN}` },
  });
  if (!res.ok) throw httpError("GET", path, res.status);
  return res.json();
}
async function apiPost(path, body) {
  const res = await fetch(`${API}${path}`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${TOKEN}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ ...body, script_version: SCRIPT_VERSION }),
  });
  if (!res.ok) throw httpError("POST", path, res.status);
  return res.json();
}

// ── SSRF defense for `extract` (defense-in-depth with the server-side
// `validate_extract_url` guard) ──────────────────────────────────────────
// The server authoritatively rejects loopback/private/link-local/metadata
// targets, but it can't see DNS-rebinding (a public name that resolves to a
// private address). The worker drives the operator's REAL logged-in Chrome,
// so re-validate here at navigation time: resolve the host and refuse any
// non-public address. Best-effort (a TOCTOU window remains before goto), but
// it closes the rebinding gap the server cannot.
function isBlockedIp(ip) {
  const v = isIP(ip);
  if (v === 4) {
    const o = ip.split(".").map(Number);
    if (o[0] === 10) return true; // 10/8 private
    if (o[0] === 127) return true; // loopback
    if (o[0] === 0) return true; // unspecified / this-network
    if (o[0] === 169 && o[1] === 254) return true; // link-local + metadata
    if (o[0] === 172 && o[1] >= 16 && o[1] <= 31) return true; // 172.16/12
    if (o[0] === 192 && o[1] === 168) return true; // 192.168/16
    if (o[0] === 100 && o[1] >= 64 && o[1] <= 127) return true; // 100.64/10 CGNAT
    if (o[0] >= 224) return true; // multicast + reserved + broadcast
    return false;
  }
  if (v === 6) {
    const a = ip.toLowerCase();
    if (a === "::" || a === "::1") return true; // unspecified / loopback
    const head = a.split(":")[0] || "";
    const b0 = parseInt(head.padStart(4, "0").slice(0, 2), 16);
    if ((b0 & 0xfe) === 0xfc) return true; // fc00::/7 unique-local
    if (b0 === 0xfe) {
      const b1 = parseInt(head.padStart(4, "0").slice(2, 4), 16);
      if ((b1 & 0xc0) === 0x80) return true; // fe80::/10 link-local
    }
    if (a.startsWith("ff")) return true; // multicast
    // IPv4-mapped ::ffff:a.b.c.d — re-check the embedded v4.
    const m = a.match(/::ffff:(\d+\.\d+\.\d+\.\d+)$/);
    if (m) return isBlockedIp(m[1]);
    return false;
  }
  return true; // not a recognizable IP → refuse
}
async function assertPublicTarget(rawUrl) {
  let u;
  try {
    u = new URL(rawUrl);
  } catch {
    throw new Error("invalid extract url");
  }
  if (u.protocol !== "http:" && u.protocol !== "https:") {
    throw new Error("extract url scheme not allowed");
  }
  const host = u.hostname.replace(/^\[|\]$/g, "");
  if (isIP(host)) {
    if (isBlockedIp(host)) throw new Error("extract target host is not allowed");
    return;
  }
  const addrs = await lookup(host, { all: true });
  if (!addrs.length) throw new Error("extract host did not resolve");
  for (const { address } of addrs) {
    if (isBlockedIp(address)) {
      throw new Error("extract target resolves to a non-public address");
    }
  }
}

// ── DOM core injected into the ChatGPT page ──────────────────────────────
// Ported from the proven userscript extractors: KaTeX/MathJax → LaTeX, the
// Pro-reasoning "still generating" probe, latest-answer + full-transcript
// extraction. Installed on window.__nyx and re-installed after navigation.
const DOM_CORE = `
window.__nyx = (function () {
  function extractTextWithMath(el) {
    if (!el) return "";
    const clone = el.cloneNode(true);
    for (const ann of Array.from(clone.querySelectorAll('annotation[encoding="application/x-tex"]'))) {
      const latex = (ann.textContent || "").trim();
      if (!latex) continue;
      const outer = ann.closest(".katex-display, .katex") || ann.parentElement;
      if (outer) {
        const disp = outer.classList.contains("katex-display") ||
          (outer.parentElement && outer.parentElement.classList.contains("katex-display"));
        outer.replaceWith(document.createTextNode(disp ? "\\n$$" + latex + "$$\\n" : " $" + latex + "$ "));
      }
    }
    for (const mjx of Array.from(clone.querySelectorAll("mjx-container"))) {
      let latex = "";
      const a = mjx.querySelector('annotation[encoding*="TeX"]');
      if (a) latex = (a.textContent || "").trim();
      if (!latex) latex = mjx.getAttribute("aria-label") || mjx.getAttribute("data-latex") || "";
      if (latex) {
        const disp = mjx.getAttribute("display") === "true" || mjx.getAttribute("data-display") === "block";
        mjx.replaceWith(document.createTextNode(disp ? "\\n$$" + latex + "$$\\n" : " $" + latex + "$ "));
      }
    }
    for (const m of Array.from(clone.querySelectorAll("math"))) {
      const alt = m.getAttribute("alttext") || "";
      if (alt) m.replaceWith(document.createTextNode(" $" + alt + "$ "));
    }
    return (clone.innerText || "").trim();
  }

  const CHROME_RE = /^(ChatGPT|You said:|ChatGPT said:|Copy code|Copy|Share|Regenerate|4o|o\\d|GPT-|Ask anything|Send a message)$/i;
  function cleanText(text) {
    return text.split("\\n").filter((line) => {
      const t = line.trim();
      if (!t) return true;
      if (CHROME_RE.test(t)) return false;
      return true;
    }).join("\\n").trim();
  }

  function isStillGenerating() {
    const dom = !!(
      document.querySelector("button[aria-label='Stop generating']") ||
      document.querySelector("button[aria-label='Stop streaming']") ||
      document.querySelector("button[aria-label='停止生成']") ||
      document.querySelector("button[data-testid='stop-button']") ||
      document.querySelector("[class*='result-streaming']") ||
      document.querySelector("[class*='streaming']") ||
      document.querySelector("[class*='thinking']") ||
      document.querySelector("[class*='reasoning']")
    );
    if (dom) return true;
    try {
      const main = document.querySelector("main");
      if (!main) return false;
      const txt = main.innerText || "";
      const pre = /Pro thinking|Extended Pro|Reasoning…/i.test(txt);
      const post = /Thought for\\s+\\d+/i.test(txt);
      if (pre && !post) return true;
    } catch (e) {}
    return false;
  }

  function assistantCount() {
    return document.querySelectorAll("[data-message-author-role='assistant']").length;
  }

  function scrollContainer() {
    const firstMessage = document.querySelector("[data-message-author-role]");
    let el = firstMessage ? firstMessage.parentElement : null;
    while (el && el !== document.body && el !== document.documentElement) {
      try {
        const style = getComputedStyle(el);
        if (
          el.scrollHeight > el.clientHeight + 4 &&
          (style.overflowY === "auto" || style.overflowY === "scroll")
        ) {
          return el;
        }
      } catch (e) {}
      el = el.parentElement;
    }
    return document.scrollingElement || document.body;
  }

  // Latest assistant message text (the answer to the last prompt).
  function extractResponse() {
    const main = document.querySelector("main");
    if (!main) return "";
    const els = main.querySelectorAll("[data-message-author-role='assistant']");
    if (!els.length) return "";
    return cleanText(extractTextWithMath(els[els.length - 1]));
  }

  // Image URLs in the LATEST assistant turn (generated images). An image-gen
  // turn renders its <img> inside a conversation-turn that does NOT carry
  // data-message-author-role="assistant" (verified against the live DOM), so
  // scope to the last conversation-turn — skipping it if it's the user's —
  // and fall back to the last assistant message otherwise. Content images are
  // matched by ChatGPT's file/CDN src patterns or a "Generated image" alt;
  // small sprites/avatars are dropped. Dedupes the thumbnail/full/zoom copies
  // that share one src.
  function extractImages() {
    const main = document.querySelector("main");
    if (!main) return [];
    let scope = null;
    const turns = main.querySelectorAll('[data-testid^="conversation-turn"]');
    if (turns.length) {
      scope = turns[turns.length - 1];
      if (scope.querySelector("[data-message-author-role='user']")) return [];
    } else {
      const els = main.querySelectorAll("[data-message-author-role='assistant']");
      if (!els.length) return [];
      scope = els[els.length - 1];
    }
    const out = [];
    const seen = new Set();
    for (const img of Array.from(scope.querySelectorAll("img"))) {
      const src = img.currentSrc || img.src || "";
      if (!src || !/^(https?:|blob:)/.test(src)) continue;
      // SSRF guard: the assistant turn is untrusted output, so only ever fetch
      // ChatGPT's own content hosts — never a model-emitted <img src> pointing
      // at an arbitrary/internal URL. Same allowlist downloadImages fetches.
      const looksContent =
        /oaiusercontent|backend-api|blob:/.test(src) ||
        /^generated image/i.test(img.alt || "");
      if (!looksContent) continue;
      const w = img.naturalWidth || img.width || 0;
      const h = img.naturalHeight || img.height || 0;
      if (w && h && (w < 64 || h < 64)) continue;
      // Dedupe by file id when present, so one generated image rendered at
      // multiple resolutions (thumbnail/full/zoom) under different URLs
      // collapses to a single entry; fall back to the exact src.
      const idMatch = src.match(/file[-_][A-Za-z0-9]+/);
      const key = idMatch ? idMatch[0] : src;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(src);
    }
    return out;
  }

  // Full conversation: every user/assistant turn in order.
  function extractTranscript() {
    const main = document.querySelector("main") || document.body;
    const nodes = main.querySelectorAll("[data-message-author-role]");
    const turns = [];
    for (const el of nodes) {
      const role = el.getAttribute("data-message-author-role");
      if (role !== "user" && role !== "assistant") continue;
      const text = cleanText(extractTextWithMath(el));
      if (text) turns.push({ role, text });
    }
    return turns;
  }

  function extractTranscriptKeys() {
    const main = document.querySelector("main") || document.body;
    const nodes = Array.from(main.querySelectorAll("[data-message-author-role]"));
    const turns = [];
    let fallbackIndex = 0;
    for (const el of nodes) {
      const role = el.getAttribute("data-message-author-role");
      if (role !== "user" && role !== "assistant") continue;
      const turn = el.closest('[data-testid^="conversation-turn"]');
      const testid = turn ? turn.getAttribute("data-testid") : "";
      let key = testid || role + "#" + fallbackIndex++;
      const text = cleanText(extractTextWithMath(el));
      if (!text) continue;
      if (!testid) key = key + "|" + text;
      turns.push({ key, role, text });
    }
    return { rendered: nodes.length, turns };
  }

  return { isStillGenerating, assistantCount, extractResponse, extractImages, extractTranscript, extractTranscriptKeys, scrollContainer, extractTextWithMath, cleanText };
})();
`;

async function installDomCore(page) {
  // applies on future navigations…
  await page.addInitScript({ content: DOM_CORE });
  // …and right now.
  try {
    await page.evaluate(DOM_CORE);
  } catch (e) {
    /* page mid-navigation; addInitScript covers the next load */
  }
}

// ── ChatGPT tab acquisition ──────────────────────────────────────────────
function isChatGptUrl(u) {
  return /https:\/\/(chatgpt\.com|chat\.openai\.com)\//.test(u || "");
}

async function getChatPage(context) {
  let page = context.pages().find((p) => isChatGptUrl(p.url()));
  if (!page) {
    page = await context.newPage();
    await page.goto("https://chatgpt.com/", { waitUntil: "domcontentloaded" });
  }
  await installDomCore(page);
  return page;
}

// ── Prompt flow ──────────────────────────────────────────────────────────
function normalizeModelLabel(label) {
  return (label || "")
    .toLowerCase()
    .trim()
    .replace(/^(chatgpt|openai)-/, "")
    .replace(/-(pro|extended)$/g, "")
    .replace(/[\s.-]+/g, "");
}

async function clickFirstVisible(locator, timeout = 5000) {
  const count = await locator.count();
  for (let i = 0; i < count; i++) {
    const item = locator.nth(i);
    try {
      await item.click({ timeout });
      return true;
    } catch (e) {}
  }
  return false;
}

async function waitForModelMenu(page, timeout = 5000) {
  try {
    await page.locator('[role="menu"], [role="listbox"]').first().waitFor({ state: "visible", timeout });
    return true;
  } catch (e) {
    return false;
  }
}

async function clickMatchingModelItem(page, wanted) {
  const items = page.locator('[role="menuitem"], [role="option"]');
  const count = await items.count();
  for (let i = 0; i < count; i++) {
    const item = items.nth(i);
    let text = "";
    try {
      if (!(await item.isVisible())) continue;
      text = (await item.innerText({ timeout: 1000 })).trim();
    } catch (e) {
      continue;
    }
    const candidate = normalizeModelLabel(text);
    if (!candidate) continue;
    if (candidate.includes(wanted) || wanted.includes(candidate)) {
      await item.click({ timeout: 5000 });
      return text || candidate;
    }
  }
  return null;
}

async function selectModel(page, modelLabel) {
  try {
    await page.bringToFront().catch(() => {});
    const rawLabel = (modelLabel || "").trim();
    const wanted = normalizeModelLabel(rawLabel);
    if (!wanted) return;

    const target = await page.evaluate((label) => {
      const raw = (label || "").trim();
      const lower = raw.toLowerCase();
      const compact = lower
        .replace(/^(chatgpt|openai)-/, "")
        .replace(/[\s._-]+/g, "");
      if (lower.includes("pro")) return "Pro 扩展";
      if (/极速|fast/.test(lower)) return "极速";
      if (/均衡|balanced/.test(lower)) return "均衡";
      if (/高级|advanced/.test(lower)) return "高级";
      if (/超高|ultra/.test(lower)) return "超高";
      if (/扩展|extended/.test(lower)) return "Pro 扩展";
      if (/gpt[\s-]*5(\.5)?\b/.test(lower) || /\b5\.5\b/.test(lower) || compact === "gpt55" || compact === "gpt5") {
        return "GPT-5.5";
      }
      return raw;
    }, rawLabel);

    log(`selecting model "${modelLabel}"`);
    const opened = await page.evaluate(() => {
      try {
        const visible = (el) => {
          const r = el.getBoundingClientRect();
          const style = getComputedStyle(el);
          return r.width > 0 && r.height > 0 && style.visibility !== "hidden" && style.display !== "none";
        };
        let picker = document.querySelector('button.__composer-pill[aria-haspopup="menu"]');
        if (!picker || !visible(picker)) {
          picker = Array.from(document.querySelectorAll('button[aria-haspopup="menu"]')).find((btn) => {
            if (!visible(btn)) return false;
            const text = (btn.innerText || btn.textContent || "").trim();
            return text.length > 0 &&
              text.length < 30 &&
              /pro|gpt|思考|扩展|极速|均衡|高级|超高|\b5(\.|\b)/i.test(text);
          });
        }
        if (!picker) return false;
        picker.click();
        return true;
      } catch (e) {
        return false;
      }
    });

    if (!opened || !(await waitForModelMenu(page, 5000))) {
      log(`model picker unavailable for "${modelLabel}", using current`);
      return;
    }

    const clickMatch = async () => page.evaluate(({ label, resolvedTarget }) => {
      try {
        const normalize = (value) => (value || "")
          .toLowerCase()
          .trim()
          .replace(/^(chatgpt|openai)-/, "")
          .replace(/[\s._-]+/g, "");
        const rawNeedle = (label || "").trim();
        const rawTarget = (resolvedTarget || "").trim();
        const wantedValues = Array.from(new Set([
          normalize(rawNeedle),
          normalize(rawTarget),
        ].filter(Boolean)));
        const directValues = [rawNeedle.toLowerCase(), rawTarget.toLowerCase()].filter(Boolean);
        const visible = (el) => {
          const r = el.getBoundingClientRect();
          const style = getComputedStyle(el);
          return r.width > 0 && r.height > 0 && style.visibility !== "hidden" && style.display !== "none";
        };
        const items = Array.from(document.querySelectorAll('[role="menuitemradio"],[role="menuitem"],[role="option"]'));
        for (const item of items) {
          if (!visible(item)) continue;
          const text = (item.innerText || item.textContent || "").trim();
          if (!text) continue;
          const candidate = normalize(text);
          const direct = text.toLowerCase();
          const matched = wantedValues.some((wanted) => candidate === wanted || candidate.includes(wanted) || wanted.includes(candidate)) ||
            directValues.some((wanted) => direct === wanted || direct.includes(wanted) || wanted.includes(direct));
          if (!matched) continue;
          const role = item.getAttribute("role") || "";
          item.click();
          return { text, role };
        }
      } catch (e) {}
      return null;
    }, { label: rawLabel, resolvedTarget: target });

    let directMatch = await clickMatch();
    if (directMatch && directMatch.role === "menuitem" && normalizeModelLabel(target) === "gpt55") {
      await sleep(600);
      directMatch = (await clickMatch()) || directMatch;
    }
    if (directMatch) {
      log(`model set to "${target}"`);
      return;
    }

    const openedEffortSubmenu = await page.evaluate(() => {
      try {
        const trigger = document.querySelector('[data-testid="composer-intelligence-pro-thinking-effort-trigger"]');
        if (!trigger) return false;
        trigger.click();
        return true;
      } catch (e) {
        return false;
      }
    });
    if (openedEffortSubmenu) {
      await sleep(600);
      directMatch = await clickMatch();
      if (directMatch) {
        log(`model set to "${target}"`);
        return;
      }
    }

    await page.keyboard.press("Escape");
    log(`model "${modelLabel}" not found in picker, using current`);
  } catch (err) {
    try {
      await page.keyboard.press("Escape");
    } catch (e) {}
    log(`model "${modelLabel}" selection failed: ${err.message}; using current`);
  }
}

async function uploadPdf(page, task_id, task) {
  if (!task.pdf_base64) return false;
  const buffer = Buffer.from(task.pdf_base64, "base64");
  const name = task.pdf_name || "attachment.pdf";
  log(`uploading PDF ${name} (${(buffer.length / 1024).toFixed(0)} KB)`);
  let fileInput = page.locator("input[type='file']").first();
  if ((await fileInput.count()) === 0) {
    const attach = page.locator("button[aria-label='Attach files'], button[aria-label='Upload file'], button[data-testid='composer-attach-button'], button[aria-haspopup='menu']").first();
    if (await attach.count()) { await attach.click().catch(() => {}); await sleep(800); }
    fileInput = page.locator("input[type='file']").first();
  }
  try {
    await fileInput.setInputFiles({ name, mimeType: "application/pdf", buffer }, { timeout: 30000 });
  } catch (e) { log(`PDF setInputFiles failed: ${e.message}`); return false; }
  const start = Date.now();
  let lastHeartbeat = start;
  while (Date.now() - start < 120000) {
    await sleep(1500);
    // Keep the task lease warm + honor a server-side cancel during a long upload
    // (matches the heartbeat discipline in waitForResponse).
    if (Date.now() - lastHeartbeat >= HEARTBEAT_MS) {
      lastHeartbeat = Date.now();
      if (await ack(task_id, "uploading_pdf")) throw new Error("cancelled by server");
    }
    // Primary signal: the filename rendered in the composer attachment chip
    // (verified against the live ChatGPT DOM). The data-testid*='file' / class
    // fallbacks cover future DOM tweaks.
    const { attached, uploading } = await page.evaluate((fname) => {
      const txt = document.body.innerText || "";
      return {
        attached: txt.includes(fname)
          || !!document.querySelector("[data-testid*='file'],[class*='file-chip'],[class*='attachment']")
          || !!document.querySelector("img[alt*='pdf' i]"),
        uploading: !!document.querySelector("[role='progressbar'],[class*='uploading']"),
      };
    }, name);
    if (attached && !uploading) { log(`PDF attached (${Math.round((Date.now()-start)/1000)}s)`); return true; }
  }
  log("PDF upload wait timed out — sending anyway");
  return false;
}

async function handlePrompt(page, task) {
  const { task_id } = task;
  log(`prompt task ${task_id} (followup=${!!task.is_followup})`);
  await page.bringToFront().catch(() => {});

  // Navigate: continue an existing conversation, or start a FRESH chat.
  // For a fresh prompt we must leave any /c/<uuid> page we're parked on,
  // otherwise we'd type into the previous conversation.
  let navTarget = null;
  const onConvPage = /\/c\/[a-f0-9-]{6,}/.test(page.url());
  if (task.is_followup && task.conversation_url) {
    const cid = convId(task.conversation_url);
    if (!cid || !page.url().includes(cid)) navTarget = task.conversation_url;
  } else {
    const base = task.required_project_url || "https://chatgpt.com/";
    if (onConvPage || !page.url().startsWith(base)) navTarget = base;
  }
  if (navTarget) {
    await page.goto(navTarget, { waitUntil: "domcontentloaded" });
    await installDomCore(page);
    await page.bringToFront().catch(() => {});
    await sleep(2500);
  }

  await ack(task_id, "page_ready");

  if (task.model && task.model !== "unknown") {
    await ack(task_id, "selecting_model");
    await selectModel(page, task.model);
  }

  // Type the prompt into the composer (native — more robust than the
  // userscript's execCommand fallbacks) and send.
  const input = page
    .locator("#prompt-textarea, div[contenteditable='true'][role='textbox'], textarea[data-testid='prompt-textarea']")
    .first();
  await input.waitFor({ state: "visible", timeout: 60000 });
  await input.click();
  await input.fill(task.prompt);
  await sleep(300);
  // Only attach a PDF on the FIRST turn of a conversation — never re-upload it
  // into an existing chat if the server ever resends pdf_base64 on a follow-up
  // (mirrors the userscript's `!is_followup && pdf_base64` guard).
  if (!task.is_followup && task.pdf_base64) {
    await ack(task_id, "uploading_pdf");
    await uploadPdf(page, task_id, task);
  }

  const beforeCount = await page.evaluate(() => window.__nyx.assistantCount());
  const sendBtn = page
    .locator("button[data-testid='send-button'], button[aria-label='Send prompt'], button[aria-label='发送提示']")
    .first();
  await sendBtn.click({ timeout: 30000 });
  await ack(task_id, "sent");

  const { text, images } = await waitForResponse(page, task_id, beforeCount);
  const chatgpt_url = page.url();
  const downloaded = await downloadImages(page, images);
  // Image-only turns settle with empty text but a non-empty image list — still
  // a success. Only an empty text AND no images is an extraction failure.
  if ((!text || !text.trim()) && downloaded.length === 0) {
    await apiPost("/result", { task_id, worker: LABEL, response: "ERROR: empty extraction", chatgpt_url, model: task.model });
    log(`prompt ${task_id} → empty`);
    return;
  }
  const res = await apiPost("/result", { task_id, worker: LABEL, response: text || "", images: downloaded, chatgpt_url, model: task.model });
  log(`prompt ${task_id} → ${res.status} (${(text || "").length} chars, ${downloaded.length} image(s))`);
}

function convId(url) {
  const m = (url || "").match(/\/c\/([a-f0-9-]{6,})/);
  return m ? m[1] : null;
}

// Returns { text, images } where images is an array of on-page src strings
// for the latest assistant turn. An image-generation turn settles with empty
// text but a non-empty images list; the stability key spans both so the turn
// terminates once text AND image srcs stop changing.
async function waitForResponse(page, task_id, beforeCount) {
  const start = Date.now();
  let lastHeartbeat = start;
  let lastKey = "";
  let stable = 0;
  while (Date.now() - start < MAX_WAIT_MS) {
    await sleep(STABLE_INTERVAL_MS);
    if (Date.now() - lastHeartbeat >= HEARTBEAT_MS) {
      lastHeartbeat = Date.now();
      const cancelled = await ack(task_id, "waiting_response");
      if (cancelled) throw new Error("cancelled by server");
    }
    const [generating, count, text, images] = await page.evaluate(() => [
      window.__nyx.isStillGenerating(),
      window.__nyx.assistantCount(),
      window.__nyx.extractResponse(),
      window.__nyx.extractImages(),
    ]);
    const hasText = !!(text && text.length > 0);
    const hasImages = Array.isArray(images) && images.length > 0;
    // A text answer bumps the assistant-role count; an image-generation turn
    // does NOT (its <img> lives in a conversation-turn with no assistant role),
    // so images carry that case through. Until text or an image appears there's
    // no new answer yet — wedge guard bails fast if ChatGPT has clearly stopped.
    if (count <= beforeCount && !hasImages) {
      if (!generating && Date.now() - start >= NO_OUTPUT_IDLE_MS) {
        throw new Error("no assistant output (idle timeout)");
      }
      continue;
    }
    if (generating) {
      stable = 0;
      continue;
    }
    if (!hasText && !hasImages) {
      // New turn settled but produced nothing extractable (e.g. an unrenderable
      // tool turn). Don't wedge — fail fast once the idle window elapses.
      if (Date.now() - start >= NO_OUTPUT_IDLE_MS) {
        throw new Error("assistant turn produced no extractable content (idle timeout)");
      }
      stable = 0;
      continue;
    }
    const key =
      (text || "").slice(0, 200) + "|" + (text || "").length + "|" + (images || []).join(",");
    if (key === lastKey) {
      stable += 1;
      if (stable >= 2) return { text: text || "", images: images || [] };
    } else {
      stable = 0;
      lastKey = key;
    }
  }
  // Timed out. Only return content if a NEW assistant turn actually appeared
  // since we sent the prompt; otherwise the latest message is stale (a previous
  // turn), so return empty and let the server mark the task failed instead of
  // handing back the wrong answer.
  const [count, text, images] = await page.evaluate(() => [
    window.__nyx.assistantCount(),
    window.__nyx.extractResponse(),
    window.__nyx.extractImages(),
  ]);
  return count > beforeCount
    ? { text: text || "", images: images || [] }
    : { text: "", images: [] };
}

// Download the latest turn's images through the browser's authenticated
// context — page.request.get carries the session cookies and isn't subject to
// the same-origin policy, so it can read cross-origin oaiusercontent bytes a
// page fetch() would get only as an opaque (unreadable) response. blob: URLs
// resolve only inside the page, so those are fetched there. Returns the
// worker-API image array; caps mirror the server (which re-validates).
async function downloadImages(page, srcs) {
  const out = [];
  let total = 0;
  for (const src of (srcs || []).slice(0, MAX_IMAGES)) {
    // Trust boundary (SSRF): this is where the privileged, cookie-bearing fetch
    // happens, so enforce the content-host allowlist HERE — independent of
    // extractImages' heuristics (its alt-based match is model-controlled and
    // could otherwise smuggle an internal URL through). blob: is page-local.
    if (!src.startsWith("blob:") && !/oaiusercontent|backend-api/.test(src)) continue;
    try {
      let buffer, mime;
      if (src.startsWith("blob:")) {
        const data = await page.evaluate(async (u) => {
          const r = await fetch(u);
          const b = await r.blob();
          const buf = new Uint8Array(await b.arrayBuffer());
          let bin = "";
          for (let i = 0; i < buf.length; i++) bin += String.fromCharCode(buf[i]);
          return { b64: btoa(bin), mime: b.type || "image/png" };
        }, src);
        buffer = Buffer.from(data.b64, "base64");
        mime = data.mime;
      } else {
        const resp = await page.request.get(src, { timeout: 30000 });
        if (!resp.ok()) {
          log(`image fetch ${resp.status()} for ${src.slice(0, 80)}`);
          continue;
        }
        buffer = await resp.body();
        mime = (resp.headers()["content-type"] || "image/png").split(";")[0].trim();
      }
      if (!buffer || !buffer.length) continue;
      if (buffer.length > MAX_IMAGE_BYTES) {
        log(`image too large (${buffer.length}B), skipping`);
        continue;
      }
      if (total + buffer.length > MAX_IMAGES_TOTAL_BYTES) {
        log("image total cap reached, skipping remaining");
        break;
      }
      total += buffer.length;
      if (!/^image\//.test(mime)) mime = "image/png";
      const ext = (mime.split("/")[1] || "png").replace(/[^a-z0-9]/gi, "") || "png";
      out.push({ mime, name: `image_${out.length + 1}.${ext}`, data_base64: buffer.toString("base64") });
    } catch (e) {
      log(`image download failed: ${e.message}`);
    }
  }
  return out;
}

// ── Scrape flow (attach existing conversation) ───────────────────────────
async function loadFullTranscript(page) {
  let renderedCount = 0;
  const renderStart = Date.now();
  while (Date.now() - renderStart < 20000) {
    renderedCount = await page.evaluate(() => document.querySelectorAll("[data-message-author-role]").length);
    if (renderedCount > 0) break;
    await sleep(700);
  }
  await sleep(1500);

  await expandCollapsibles(page);

  const result = await page.evaluate(async () => {
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const nyx = window.__nyx || {};
    const clean = (text) => nyx.cleanText ? nyx.cleanText(text || "") : (text || "").trim();
    const extract = (el) => nyx.extractTextWithMath ? nyx.extractTextWithMath(el) : ((el && el.innerText) || "");
    const wraps = Array.from(document.querySelectorAll('[data-testid^="conversation-turn"]')).slice(0, 2000);

    if (wraps.length > 0) {
      const turns = [];
      const seen = new Set();
      for (const w of wraps) {
        try {
          w.scrollIntoView({ block: "center" });
        } catch (e) {}
        await sleep(150);
        const roleEl = w.querySelector("[data-message-author-role]");
        if (!roleEl) continue;
        const role = roleEl.getAttribute("data-message-author-role");
        if (role !== "user" && role !== "assistant") continue;
        const key = w.getAttribute("data-testid");
        if (!key || seen.has(key)) continue;
        const text = clean(extract(roleEl)).slice(0, 200000);
        if (!text) continue;
        seen.add(key);
        turns.push({ role, text });
      }
      return { rendered: wraps.length, turns };
    }

    let lastHeight = -1;
    let stableHeight = 0;
    for (let i = 0; i < 50; i++) {
      try {
        const sc = nyx.scrollContainer();
        sc.scrollTop = 0;
      } catch (e) {}
      await sleep(700);
      let height = 0;
      try {
        const sc = nyx.scrollContainer();
        height = sc.scrollHeight || 0;
      } catch (e) {}
      if (height === lastHeight) {
        stableHeight += 1;
        if (stableHeight >= 3) break;
      } else {
        stableHeight = 0;
        lastHeight = height;
      }
    }

    const acc = new Map();
    const order = [];
    let rendered = document.querySelectorAll("[data-message-author-role]").length;
    let bottomStable = 0;
    for (let i = 0; i < 120 && acc.size < 2000; i++) {
      const snapshot = nyx.extractTranscriptKeys();
      rendered = Math.max(rendered, snapshot.rendered || 0);
      for (const turn of snapshot.turns || []) {
        const text = (turn.text || "").slice(0, 200000);
        if (!text) continue;
        if (!acc.has(turn.key)) order.push(turn.key);
        acc.set(turn.key, { role: turn.role, text });
        if (acc.size >= 2000) {
          break;
        }
      }

      try {
        const sc = nyx.scrollContainer();
        const step = Math.floor((sc.clientHeight || window.innerHeight || 800) * 0.8);
        sc.scrollTop = Math.min(sc.scrollHeight, sc.scrollTop + step);
      } catch (e) {}
      await sleep(600);
      let atBottom = false;
      try {
        const sc = nyx.scrollContainer();
        atBottom = sc.scrollTop + sc.clientHeight >= sc.scrollHeight - 4;
      } catch (e) {}
      if (atBottom) {
        bottomStable += 1;
        if (bottomStable >= 2) break;
      } else {
        bottomStable = 0;
      }
    }
    return { rendered, turns: order.map((key) => acc.get(key)).filter(Boolean) };
  });

  const turns = result.turns || [];
  renderedCount = Math.max(renderedCount, result.rendered || 0);
  log(`scrape: rendered≈${renderedCount} turns, accumulated ${turns.length}`);
  return turns;
}

async function handleScrape(page, task) {
  const { task_id, conversation_url } = task;
  log(`scrape task ${task_id} → ${conversation_url}`);
  await page.bringToFront().catch(() => {});
  if (!conversation_url) {
    await apiPost("/transcript", { task_id, worker: LABEL, turns: [], chatgpt_url: page.url() });
    return;
  }
  await page.goto(conversation_url, { waitUntil: "domcontentloaded" });
  await installDomCore(page);
  await page.bringToFront().catch(() => {});
  await ack(task_id, "scraping");

  const turns = await loadFullTranscript(page);
  const res = await apiPost("/transcript", { task_id, worker: LABEL, turns, chatgpt_url: page.url() });
  log(`scrape ${task_id} → ${res.status} (${turns.length} turns, ${res.imported_pairs} pairs)`);
}

// ── General web extraction flow ──────────────────────────────────────────
async function scrollLazyPage(page) {
  let lastHeight = -1;
  let stableHeight = 0;
  for (let i = 0; i < 6; i++) {
    const height = await page.evaluate(() => {
      const sc = document.scrollingElement || document.documentElement || document.body;
      const before = sc ? sc.scrollHeight : document.body.scrollHeight;
      try {
        if (sc) sc.scrollTop = before;
        else window.scrollTo(0, before);
      } catch (e) {
        try { window.scrollTo(0, before); } catch (inner) {}
      }
      return before || 0;
    });
    await sleep(600);
    const nextHeight = await page.evaluate(() => {
      const sc = document.scrollingElement || document.documentElement || document.body;
      return (sc && sc.scrollHeight) || document.body.scrollHeight || 0;
    });
    if (nextHeight === lastHeight || nextHeight === height) {
      stableHeight += 1;
      if (stableHeight >= 2) break;
    } else {
      stableHeight = 0;
    }
    lastHeight = nextHeight;
  }
}

async function expandCollapsibles(page) {
  try {
    await page.evaluate(() => {
      try {
        const root = document.querySelector("main") || document.body;
        if (!root) return;
        const isVisible = (el) => {
          const r = el.getBoundingClientRect();
          const style = getComputedStyle(el);
          return r.width > 0 && r.height > 0 && style.visibility !== "hidden" && style.display !== "none";
        };
        const inComposerOrChrome = (el) => {
          const text = (el.innerText || el.textContent || "").trim();
          if (el.closest("#prompt-textarea, form, textarea, [contenteditable='true'][role='textbox'], [class*='composer'], [data-testid='composer'], [data-testid='send-button'], [data-testid='stop-button']")) {
            return true;
          }
          if (el.matches("button.__composer-pill, button[aria-haspopup='menu'], button[data-testid='send-button'], button[data-testid='stop-button']")) {
            return true;
          }
          if (/^(Send|Stop|发送|停止|GPT-|Pro|极速|均衡|高级|超高)$/i.test(text)) return true;
          return false;
        };
        let clicked = 0;
        for (const detail of Array.from(root.querySelectorAll("details:not([open])"))) {
          if (clicked >= 40) break;
          try {
            detail.open = true;
            clicked += 1;
          } catch (e) {}
        }
        const candidates = Array.from(root.querySelectorAll('[aria-expanded="false"], button, [role="button"]'));
        for (const el of candidates) {
          if (clicked >= 40) break;
          try {
            if (!isVisible(el) || inComposerOrChrome(el)) continue;
            const text = (el.innerText || el.textContent || el.getAttribute("aria-label") || "").trim();
            const collapsed = el.getAttribute("aria-expanded") === "false";
            const looksExpandable = collapsed || /Thought for|思考|显示更多|Show more|展开/i.test(text);
            if (!looksExpandable) continue;
            el.click();
            clicked += 1;
          } catch (e) {}
        }
      } catch (e) {}
    });
    await sleep(300);
  } catch (e) {}
}

async function handleExtract(page, task) {
  const { task_id } = task;
  let targetHost = "-";
  try {
    targetHost = new URL(task.target_url).host || "-";
  } catch (e) {}
  log(`extract task ${task_id} → host=${targetHost}`);
  try {
    // Defense-in-depth SSRF check at navigation time (catches DNS rebinding
    // the server-side guard can't see); explicit timeout so a slow/hostile
    // URL can't stall this single worker page.
    await assertPublicTarget(task.target_url);
    await page.goto(task.target_url, {
      waitUntil: "domcontentloaded",
      timeout: 30000,
    });
    await page.bringToFront().catch(() => {});
    await page.waitForLoadState("networkidle", { timeout: 8000 }).catch(() => {});
    await ack(task_id, "extracting");
    await scrollLazyPage(page);
    await expandCollapsibles(page);
    const content = await page.evaluate(() => {
      const root = document.querySelector("main, article") || document.body;
      return ((root && root.innerText) || "").trim().slice(0, 200000);
    });
    const response = content || "ERROR: empty extraction";
    const res = await apiPost("/result", {
      task_id,
      worker: LABEL,
      response,
      chatgpt_url: page.url(),
      model: task.model,
    });
    log(`extract ${task_id} → ${res.status} (${content.length} chars)`);
  } catch (err) {
    await apiPost("/result", {
      task_id,
      worker: LABEL,
      response: `ERROR: ${err.message}`,
      chatgpt_url: page.url(),
      model: task.model,
    });
  }
}

async function ack(task_id, phase) {
  try {
    const r = await apiPost("/ack", { task_id, worker: LABEL, phase });
    return r.status === "cancelled";
  } catch (e) {
    return false;
  }
}

// ── Main loop ────────────────────────────────────────────────────────────
async function main() {
  log(`connecting to Chrome at ${CDP_URL} …`);
  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0] || (await browser.newContext());
  let page = await getChatPage(context);
  log(`attached. worker=${LABEL} pool=${BASE_URL}. polling…`);

  for (;;) {
    try {
      if (page.isClosed()) page = await getChatPage(context);
      const resp = await apiGet(
        `/task?worker=${encodeURIComponent(LABEL)}&script_version=${SCRIPT_VERSION}&page_url=${encodeURIComponent(page.url())}`
      );
      if (resp.status === "task" && resp.task_id) {
        try {
          if (resp.kind === "scrape") await handleScrape(page, resp);
          else if (resp.kind === "extract") await handleExtract(page, resp);
          else await handlePrompt(page, resp);
        } catch (err) {
          log(`task ${resp.task_id} errored: ${err.message}`);
          // Report the failure so the task doesn't hang until lease expiry.
          try {
            if (resp.kind === "scrape") {
              await apiPost("/transcript", { task_id: resp.task_id, worker: LABEL, turns: [], chatgpt_url: page.url() });
            } else {
              await apiPost("/result", { task_id: resp.task_id, worker: LABEL, response: `ERROR: ${err.message}`, chatgpt_url: page.url(), model: resp.model });
            }
          } catch (e) {}
        }
      }
    } catch (err) {
      if (err.status === 401 || err.status === 403) {
        // Distinct, loud signal: a revoked/invalid worker token (or an
        // inactive pool) otherwise loops quietly forever. Back off hard so
        // we don't hammer the server while still recovering if the token is
        // rotated back.
        log(
          `AUTH FAILED (HTTP ${err.status}): worker token rejected. Verify NYXID_WORKER_TOKEN and that the pool is active. Backing off…`
        );
        await sleep(Math.max(POLL_MS, 30000));
        continue;
      }
      log(`poll error: ${err.message}`);
    }
    await sleep(POLL_MS);
  }
}

main().catch((e) => {
  console.error("fatal:", e);
  process.exit(1);
});
