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

const BASE_URL = (process.env.NYXID_BASE_URL || "").replace(/\/$/, "");
const TOKEN = process.env.NYXID_WORKER_TOKEN || "";
const LABEL = process.env.NYXID_WORKER_LABEL || "tab_1";
const CDP_URL = process.env.CHROME_CDP_URL || "http://localhost:9222";
const SCRIPT_VERSION = "cdp-1.0";
const POLL_MS = Number(process.env.NYXID_POLL_MS || 5000);
const STABLE_INTERVAL_MS = 8000;
const MAX_WAIT_MS = Number(process.env.NYXID_MAX_WAIT_MS || 2 * 60 * 60 * 1000); // 2h
const HEARTBEAT_MS = 60000;

if (!BASE_URL || !TOKEN) {
  console.error(
    "Missing config. Set NYXID_BASE_URL and NYXID_WORKER_TOKEN (the pool worker token, nyx_owk_...)."
  );
  process.exit(1);
}

const API = `${BASE_URL}/api/v1/oracle/worker`;

function log(msg) {
  console.log(`[nyxid-cdp ${new Date().toISOString()}] ${msg}`);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ── NyxID worker API (Bearer worker token) ───────────────────────────────
async function apiGet(path) {
  const res = await fetch(`${API}${path}`, {
    headers: { Authorization: `Bearer ${TOKEN}` },
  });
  if (!res.ok) throw new Error(`GET ${path} → ${res.status}`);
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
  if (!res.ok) throw new Error(`POST ${path} → ${res.status}`);
  return res.json();
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

  return { isStillGenerating, assistantCount, extractResponse, extractTranscript, scrollContainer };
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
    const wanted = normalizeModelLabel(modelLabel);
    if (!wanted) return;

    log(`selecting model "${modelLabel}"`);
    const testIdPicker = page.locator('button[data-testid="model-switcher-dropdown-button"]').first();
    const ariaPicker = page.locator('button[aria-label*="model" i]').first();
    const topTextPicker = page
      .locator("main button, header button, nav button, body > div button")
      .filter({ hasText: /gpt|o\d|chatgpt/i })
      .first();

    let opened = false;
    for (const picker of [testIdPicker, ariaPicker, topTextPicker]) {
      try {
        await picker.click({ timeout: 5000 });
        opened = await waitForModelMenu(page, 5000);
        if (opened) break;
      } catch (e) {}
    }

    if (!opened) {
      log(`model picker unavailable for "${modelLabel}", using current`);
      return;
    }

    const directMatch = await clickMatchingModelItem(page, wanted);
    if (directMatch) {
      log(`model "${modelLabel}" selected via picker item "${directMatch}"`);
      return;
    }

    const submenus = page
      .locator('[role="menuitem"], [role="option"], button')
      .filter({ hasText: /more models|legacy models/i });
    if (await clickFirstVisible(submenus, 3000)) {
      await waitForModelMenu(page, 5000);
      const submenuMatch = await clickMatchingModelItem(page, wanted);
      if (submenuMatch) {
        log(`model "${modelLabel}" selected via picker item "${submenuMatch}"`);
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

async function handlePrompt(page, task) {
  const { task_id } = task;
  log(`prompt task ${task_id} (followup=${!!task.is_followup})`);

  // Navigate: continue an existing conversation, or start a FRESH chat.
  // For a fresh prompt we must leave any /c/<uuid> page we're parked on,
  // otherwise we'd type into the previous conversation.
  let navTarget = null;
  const onConvPage = /\/c\/[a-f0-9-]{6,}/.test(page.url());
  if (task.is_followup && task.conversation_url) {
    if (!page.url().includes(convId(task.conversation_url))) navTarget = task.conversation_url;
  } else {
    const base = task.required_project_url || "https://chatgpt.com/";
    if (onConvPage || !page.url().startsWith(base)) navTarget = base;
  }
  if (navTarget) {
    await page.goto(navTarget, { waitUntil: "domcontentloaded" });
    await installDomCore(page);
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

  const beforeCount = await page.evaluate(() => window.__nyx.assistantCount());
  const sendBtn = page
    .locator("button[data-testid='send-button'], button[aria-label='Send prompt'], button[aria-label='发送提示']")
    .first();
  await sendBtn.click({ timeout: 30000 });
  await ack(task_id, "sent");

  const response = await waitForResponse(page, task_id, beforeCount);
  const chatgpt_url = page.url();
  if (!response || !response.trim()) {
    await apiPost("/result", { task_id, worker: LABEL, response: "ERROR: empty extraction", chatgpt_url, model: task.model });
    log(`prompt ${task_id} → empty`);
    return;
  }
  const res = await apiPost("/result", { task_id, worker: LABEL, response, chatgpt_url, model: task.model });
  log(`prompt ${task_id} → ${res.status} (${response.length} chars)`);
}

function convId(url) {
  const m = (url || "").match(/\/c\/([a-f0-9-]{6,})/);
  return m ? m[1] : " never";
}

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
    const [generating, count, text] = await page.evaluate(() => [
      window.__nyx.isStillGenerating(),
      window.__nyx.assistantCount(),
      window.__nyx.extractResponse(),
    ]);
    if (count <= beforeCount) continue; // answer not yet appended
    if (generating) {
      stable = 0;
      continue;
    }
    const key = (text || "").slice(0, 200) + "|" + (text || "").length;
    if (key === lastKey && text && text.length > 0) {
      stable += 1;
      if (stable >= 2) return text;
    } else {
      stable = 0;
      lastKey = key;
    }
  }
  // Timed out — return whatever we have.
  return page.evaluate(() => window.__nyx.extractResponse());
}

// ── Scrape flow (attach existing conversation) ───────────────────────────
async function loadFullTranscript(page) {
  let lastHeight = -1;
  let stableHeight = 0;
  for (let i = 0; i < 50; i++) {
    await page.evaluate(() => {
      const sc = window.__nyx.scrollContainer();
      sc.scrollTop = 0;
    });
    await sleep(700);
    const height = await page.evaluate(() => {
      const sc = window.__nyx.scrollContainer();
      return sc.scrollHeight || 0;
    });
    if (height === lastHeight) {
      stableHeight += 1;
      if (stableHeight >= 3) break;
    } else {
      stableHeight = 0;
      lastHeight = height;
    }
  }

  const acc = [];
  const seen = new Set();
  let bottomStable = 0;
  for (let i = 0; i < 80 && acc.length < 1000; i++) {
    const turns = await page.evaluate(() => window.__nyx.extractTranscript());
    for (const turn of turns) {
      const text = (turn.text || "").slice(0, 200000);
      const key = `${turn.role}|${text.slice(0, 120)}`;
      if (!seen.has(key)) {
        seen.add(key);
        acc.push({ role: turn.role, text });
        if (acc.length >= 1000) break;
      }
    }

    await page.evaluate(() => {
      const sc = window.__nyx.scrollContainer();
      const step = Math.floor((sc.clientHeight || window.innerHeight || 800) * 0.8);
      sc.scrollTop = Math.min(sc.scrollHeight, sc.scrollTop + step);
    });
    await sleep(500);
    const atBottom = await page.evaluate(() => {
      const sc = window.__nyx.scrollContainer();
      return sc.scrollTop + sc.clientHeight >= sc.scrollHeight - 4;
    });
    if (atBottom) {
      bottomStable += 1;
      if (bottomStable >= 2) break;
    } else {
      bottomStable = 0;
    }
  }
  return acc;
}

async function handleScrape(page, task) {
  const { task_id, conversation_url } = task;
  log(`scrape task ${task_id} → ${conversation_url}`);
  if (!conversation_url) {
    await apiPost("/transcript", { task_id, worker: LABEL, turns: [], chatgpt_url: page.url() });
    return;
  }
  await page.goto(conversation_url, { waitUntil: "domcontentloaded" });
  await installDomCore(page);
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

async function handleExtract(page, task) {
  const { task_id } = task;
  let targetHost = "-";
  try {
    targetHost = new URL(task.target_url).host || "-";
  } catch (e) {}
  log(`extract task ${task_id} → host=${targetHost}`);
  try {
    await page.goto(task.target_url, { waitUntil: "domcontentloaded" });
    await page.waitForLoadState("networkidle", { timeout: 8000 }).catch(() => {});
    await ack(task_id, "extracting");
    await scrollLazyPage(page);
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
      log(`poll error: ${err.message}`);
    }
    await sleep(POLL_MS);
  }
}

main().catch((e) => {
  console.error("fatal:", e);
  process.exit(1);
});
