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
import {
  createDecipheriv,
  createHash,
  hkdfSync,
  randomUUID,
} from "node:crypto";
import { lookup } from "node:dns/promises";
import {
  chmodSync,
  mkdirSync,
  readFileSync,
  renameSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { isIP } from "node:net";
import { homedir } from "node:os";
import { dirname, resolve } from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

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
const BUNDLE_VERSION_FILE =
  process.env.NYXID_BUNDLE_VERSION_FILE ||
  resolve(dirname(fileURLToPath(import.meta.url)), "bundle-version");
const SOURCE_SHA256 = createHash("sha256")
  .update(readFileSync(fileURLToPath(import.meta.url)))
  .digest("hex");
const SCRIPT_VERSION = (() => {
  try {
    const version = readFileSync(BUNDLE_VERSION_FILE, "utf8").trim();
    if (
      /^[A-Za-z0-9._+-]{1,128}$/.test(version) &&
      version.endsWith(SOURCE_SHA256.slice(0, 12))
    ) {
      return version;
    }
  } catch {}
  return `cdp+${SOURCE_SHA256.slice(0, 12)}`;
})();
const POLL_MS = Number(process.env.NYXID_POLL_MS || 5000);
const STABLE_INTERVAL_MS = 8000;
const MAX_WAIT_MS = Number(process.env.NYXID_MAX_WAIT_MS || 2 * 60 * 60 * 1000); // 2h
// Wedge guard: if ChatGPT has clearly stopped (not generating) yet produced
// nothing extractable after this long, fail the task fast and free the slot
// instead of spinning to MAX_WAIT_MS. Mirrors the userscript's
// NO_OUTPUT_IDLE_TIMEOUT (420s).
const NO_OUTPUT_IDLE_MS = Number(process.env.NYXID_NO_OUTPUT_IDLE_MS || 7 * 60 * 1000);
const HEARTBEAT_MS = 60000;
const PRESENCE_MS = Number(process.env.NYXID_PRESENCE_MS || 20000);
const HTTP_TIMEOUT_MS = Number(process.env.NYXID_HTTP_TIMEOUT_MS || 30000);
const MAX_HTTP_BACKOFF_MS = Number(process.env.NYXID_MAX_HTTP_BACKOFF_MS || 60000);
const MAX_CDP_FAILURES_BEFORE_RELAUNCH = Number(
  process.env.NYXID_MAX_CDP_FAILURES_BEFORE_RELAUNCH || 3
);
const MAX_TASK_RECOVERY_FAILURES = Number(
  process.env.NYXID_MAX_TASK_RECOVERY_FAILURES || 6
);
const STATE_FILE =
  process.env.NYXID_WORKER_STATE_FILE ||
  resolve(homedir(), ".nyxid-oracle", "worker-state.json");
const INSTALLATION_ID_FILE =
  process.env.NYXID_INSTALLATION_ID_FILE ||
  resolve(dirname(STATE_FILE), "installation-id");
const CHROME_PROFILE_DIR =
  process.env.CHROME_PROFILE_DIR || resolve(homedir(), ".nyxid-oracle", "chrome-profile");
const CHROME_EXECUTABLE = process.env.NYXID_CHROME_EXECUTABLE || "";
const CHROME_DEBUG_PORT = Number(
  process.env.CHROME_DEBUG_PORT || new URL(CDP_URL).port || 9222
);
const SESSION_INFO = Buffer.from("nyxid-oracle-session-v1", "utf8");
const SESSION_AAD = SESSION_INFO;
const SESSION_FORMAT_VERSION = 1;
const MAX_SESSION_SNAPSHOT_BYTES = 512 * 1024;
const MAX_SESSION_PLAINTEXT_BYTES = 350 * 1024;
const NPM_EXECUTABLE = process.env.NYXID_NPM_EXECUTABLE || "npm";
const NPM_INSTALL_TIMEOUT_MS = Number(
  process.env.NYXID_NPM_INSTALL_TIMEOUT_MS || 5 * 60 * 1000
);
const CAPABILITIES = ["commands_v1", "upgrade_v1", "session_import_v1", "attempt_fencing_v1"];
// Result-image caps (the server re-validates and caps lower-or-equal). Kept
// below the 16 MiB worker body cap once base64-inflated (~33%).
const MAX_IMAGES = Number(process.env.NYXID_MAX_IMAGES || 4);
const MAX_IMAGE_BYTES = Number(process.env.NYXID_MAX_IMAGE_BYTES || 6 * 1024 * 1024);
const MAX_IMAGES_TOTAL_BYTES = Number(process.env.NYXID_MAX_IMAGES_TOTAL_BYTES || 9 * 1024 * 1024);

const API = `${BASE_URL}/api/v1/oracle/worker`;

function log(msg) {
  console.log(`[nyxid-cdp ${new Date().toISOString()}] ${msg}`);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export function backoffDelay(attempt, baseMs = 500, capMs = MAX_HTTP_BACKOFF_MS, random = Math.random) {
  const ceiling = Math.min(capMs, baseMs * 2 ** Math.min(Math.max(attempt, 0), 16));
  return Math.max(1, Math.floor(ceiling * (0.5 + random() * 0.5)));
}

export function normalizePromptText(value) {
  return (value || "").replace(/\s+/g, " ").trim();
}

export function decidePromptResume({ phase, prompt, turns, generating, transcriptReady, baselineTurnCount = 0 }) {
  if (!transcriptReady) return { action: "wait" };
  const wanted = normalizePromptText(prompt);
  const candidates = (turns || []).slice(Math.max(0, baselineTurnCount));
  let userIndex = -1;
  for (let i = candidates.length - 1; i >= 0; i -= 1) {
    if (
      candidates[i]?.role === "user" &&
      normalizePromptText(candidates[i]?.text) === wanted
    ) {
      userIndex = i;
      break;
    }
  }
  if (userIndex >= 0) {
    const answer = candidates
      .slice(userIndex + 1)
      .find((turn) => turn?.role === "assistant" && normalizePromptText(turn?.text));
    if (answer && !generating) return { action: "complete", response: answer.text };
    return { action: "wait" };
  }
  if (["claimed", "page_ready", "ready_to_send"].includes(phase || "claimed")) {
    return { action: "send" };
  }
  return { action: "uncertain" };
}

export function choosePromptNavigation({
  recovering,
  phase = "claimed",
  isFollowup,
  currentUrl,
  persistedUrl,
  taskConversationUrl,
  requiredProjectUrl,
}) {
  const currentConversationId = convId(currentUrl);
  const onConvPage = Boolean(currentConversationId);
  const preSend = ["claimed", "page_ready", "ready_to_send"].includes(phase);
  if (
    recovering &&
    !preSend &&
    !persistedUrl &&
    !taskConversationUrl &&
    !onConvPage
  ) {
    return { error: "recovery_conversation_unknown", target: null };
  }
  const persistedConversationId = convId(persistedUrl);
  const taskConversationId = convId(taskConversationUrl);
  const resumeUrl = persistedConversationId
    ? persistedUrl
    : taskConversationId
      ? taskConversationUrl
      : persistedUrl || taskConversationUrl;
  const resumeConversationId = persistedConversationId || taskConversationId;
  if (recovering && !preSend && onConvPage && !resumeConversationId) {
    return { error: null, target: null };
  }
  if ((isFollowup || recovering) && resumeUrl) {
    return {
      error: null,
      target:
        !resumeConversationId || currentConversationId !== resumeConversationId ? resumeUrl : null,
    };
  }
  const base = requiredProjectUrl || "https://chatgpt.com/";
  return { error: null, target: onConvPage || !currentUrl.startsWith(base) ? base : null };
}

export function taskRecoveryDecision({
  kind,
  phase,
  failureCount,
  maxFailures = MAX_TASK_RECOVERY_FAILURES,
  relaunchEvery = MAX_CDP_FAILURES_BEFORE_RELAUNCH,
}) {
  const preSend = ["claimed", "page_ready", "ready_to_send"].includes(phase || "claimed");
  if (failureCount >= maxFailures) {
    return {
      action: "fail",
      code:
        kind === "prompt" && !preSend
          ? "prompt_delivery_uncertain"
          : "browser_recovery_exhausted",
    };
  }
  return {
    action: "recover",
    forceRelaunch: relaunchEvery > 0 && failureCount % relaunchEvery === 0,
  };
}

function defaultState() {
  return {
    format_version: 1,
    instance_id: loadInstallationId(),
    draining: false,
    current_task: null,
    pending_command: null,
    pending_reports: [],
    command_results: [],
  };
}

function loadInstallationId() {
  const configured = process.env.NYXID_INSTALLATION_ID;
  if (configured && /^[A-Za-z0-9._:-]{1,128}$/.test(configured)) return configured;
  try {
    const existing = readFileSync(INSTALLATION_ID_FILE, "utf8").trim();
    if (/^[A-Za-z0-9._:-]{1,128}$/.test(existing)) return existing;
  } catch (error) {
    if (error?.code !== "ENOENT") log("installation identity file was invalid; replacing it");
  }
  const id = randomUUID();
  mkdirSync(dirname(INSTALLATION_ID_FILE), { recursive: true, mode: 0o700 });
  writeFileSync(INSTALLATION_ID_FILE, `${id}\n`, { mode: 0o600 });
  chmodSync(INSTALLATION_ID_FILE, 0o600);
  return id;
}

function loadState(path = STATE_FILE) {
  try {
    const parsed = JSON.parse(readFileSync(path, "utf8"));
    if (parsed?.format_version !== 1 || typeof parsed.instance_id !== "string") {
      throw new Error("unsupported state format");
    }
    return { ...defaultState(), ...parsed };
  } catch (error) {
    if (error?.code !== "ENOENT") {
      log("state file was invalid; starting with a fresh installation identity");
    }
    return defaultState();
  }
}

function saveState(state, path = STATE_FILE) {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const temp = `${path}.tmp-${process.pid}`;
  writeFileSync(temp, `${JSON.stringify(state)}\n`, { mode: 0o600 });
  chmodSync(temp, 0o600);
  renameSync(temp, path);
}

function updateTaskState(state, patch) {
  state.current_task = { ...(state.current_task || {}), ...patch };
  saveState(state);
}

function clearTaskState(state) {
  state.current_task = null;
  saveState(state);
}

function stableErrorCode(error) {
  if (error?.code && /^[a-z0-9_]{1,64}$/.test(error.code)) return error.code;
  if (error?.status) return `http_${error.status}`;
  const message = String(error?.message || "").toLowerCase();
  if (/target page|browser.*closed|session closed|cdp|econnrefused/.test(message)) {
    return "cdp_disconnected";
  }
  if (/timeout/.test(message)) return "operation_timeout";
  if (/fetch|network|socket|econnreset|enotfound/.test(message)) return "network_error";
  return "worker_error";
}

// ── NyxID worker API (Bearer worker token) ───────────────────────────────
function httpError(method, path, status) {
  const err = new Error(`${method} ${path} returned HTTP ${status}`);
  err.status = status;
  return err;
}

function transientHttpStatus(status) {
  return status === 408 || status === 425 || status === 429 || status >= 500;
}

async function apiRequest(method, path, body) {
  let attempt = 0;
  for (;;) {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), HTTP_TIMEOUT_MS);
    try {
      const res = await fetch(`${API}${path}`, {
        method,
        headers: {
          Authorization: `Bearer ${TOKEN}`,
          ...(body === undefined ? {} : { "Content-Type": "application/json" }),
        },
        body:
          body === undefined
            ? undefined
            : JSON.stringify({ ...body, script_version: SCRIPT_VERSION }),
        signal: controller.signal,
      });
      if (!res.ok) {
        const error = httpError(method, path, res.status);
        if (!transientHttpStatus(res.status)) throw error;
        throw Object.assign(error, { transient: true });
      }
      return await res.json();
    } catch (error) {
      if (error?.status && !error.transient) throw error;
      const delay = backoffDelay(attempt++);
      if (attempt === 1 || attempt % 5 === 0) {
        log(`NyxID unavailable (${stableErrorCode(error)}); retrying in ${delay}ms`);
      }
      await sleep(delay);
    } finally {
      clearTimeout(timeout);
    }
  }
}

const apiGet = (path) => apiRequest("GET", path);
const apiPost = (path, body) => apiRequest("POST", path, body);

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

async function detectLoggedIn(page) {
  if (!page || page.isClosed() || !isChatGptUrl(page.url())) return false;
  try {
    return await page.evaluate(() => {
      const composer = document.querySelector(
        "#prompt-textarea, div[contenteditable='true'][role='textbox'], textarea[data-testid='prompt-textarea']"
      );
      const loginLink = Array.from(document.querySelectorAll("a,button")).some((element) => {
        const text = (element.textContent || "").trim();
        return /^(log in|sign up|登录|注册)$/i.test(text);
      });
      return Boolean(composer) && !loginLink;
    });
  } catch {
    return false;
  }
}

function chromeArgs() {
  let extra = [];
  if (process.env.NYXID_CHROME_ARGS_JSON) {
    try {
      const parsed = JSON.parse(process.env.NYXID_CHROME_ARGS_JSON);
      if (Array.isArray(parsed) && parsed.every((value) => typeof value === "string")) {
        extra = parsed;
      }
    } catch {
      throw Object.assign(new Error("NYXID_CHROME_ARGS_JSON must be a JSON string array"), {
        code: "chrome_args_invalid",
      });
    }
  }
  return [
    `--remote-debugging-port=${CHROME_DEBUG_PORT}`,
    `--user-data-dir=${CHROME_PROFILE_DIR}`,
    "--no-first-run",
    "--no-default-browser-check",
    ...extra,
    "https://chatgpt.com/",
  ];
}

function launchChrome() {
  if (!CHROME_EXECUTABLE) {
    throw Object.assign(new Error("no Chrome executable configured"), {
      code: "chrome_launch_unconfigured",
    });
  }
  mkdirSync(CHROME_PROFILE_DIR, { recursive: true, mode: 0o700 });
  const child = spawn(CHROME_EXECUTABLE, chromeArgs(), {
    detached: true,
    stdio: "ignore",
  });
  child.once("error", (error) => {
    log(`Chrome launch failed (${stableErrorCode(error)})`);
  });
  child.unref();
  return child;
}

async function connectChrome(runtime) {
  const browser = await chromium.connectOverCDP(CDP_URL);
  browser.on("disconnected", () => {
    if (runtime.browser === browser) {
      runtime.browser = null;
      runtime.context = null;
      runtime.page = null;
    }
  });
  runtime.browser = browser;
  runtime.context = browser.contexts()[0] || (await browser.newContext());
  runtime.page = await getChatPage(runtime.context);
  runtime.health.cdp = 0;
  runtime.health.tab = 0;
  runtime.chromeAlive = true;
  return runtime.page;
}

async function recoverChrome(runtime, forceRelaunch = false) {
  let launched = false;
  for (;;) {
    try {
      if (forceRelaunch || runtime.health.cdp >= MAX_CDP_FAILURES_BEFORE_RELAUNCH) {
        forceRelaunch = false;
        runtime.health.cdp = 0;
        try {
          await runtime.browser?.close();
        } catch {}
        runtime.browser = null;
        runtime.context = null;
        runtime.page = null;
        launchChrome();
        launched = true;
        await sleep(1500);
      }
      return await connectChrome(runtime);
    } catch (error) {
      runtime.chromeAlive = false;
      runtime.lastError = stableErrorCode(error);
      runtime.health.cdp += 1;
      if (!launched && runtime.health.cdp >= MAX_CDP_FAILURES_BEFORE_RELAUNCH) {
        continue;
      }
      const delay = backoffDelay(runtime.health.cdp, 1000, 30000);
      log(`Chrome unavailable (${runtime.lastError}); retrying in ${delay}ms`);
      if (BASE_URL && TOKEN && Date.now() - runtime.lastPresenceAt >= PRESENCE_MS) {
        await heartbeat(runtime).catch(() => {});
      }
      await sleep(delay);
    }
  }
}

async function ensureChatPage(runtime, targetUrl) {
  if (!runtime.browser || !runtime.context || runtime.page?.isClosed()) {
    await recoverChrome(runtime);
  }
  try {
    if (!runtime.page || runtime.page.isClosed()) {
      runtime.page = await getChatPage(runtime.context);
    }
    if (targetUrl) {
      const targetConversation = convId(targetUrl);
      const currentConversation = convId(runtime.page.url());
      if (
        (targetConversation && targetConversation !== currentConversation) ||
        (!targetConversation && !runtime.page.url().startsWith(targetUrl))
      ) {
        await runtime.page.goto(targetUrl, { waitUntil: "domcontentloaded", timeout: 60000 });
        await installDomCore(runtime.page);
      }
    } else if (!isChatGptUrl(runtime.page.url())) {
      await runtime.page.goto("https://chatgpt.com/", {
        waitUntil: "domcontentloaded",
        timeout: 60000,
      });
      await installDomCore(runtime.page);
    }
    runtime.health.tab = 0;
    runtime.chromeAlive = true;
    return runtime.page;
  } catch (error) {
    runtime.health.tab += 1;
    runtime.lastError = stableErrorCode(error);
    if (runtime.health.tab >= 3) {
      runtime.health.cdp += 1;
      return recoverChrome(runtime, runtime.health.tab >= 5);
    }
    try {
      runtime.page = await runtime.context.newPage();
      await runtime.page.goto(targetUrl || "https://chatgpt.com/", {
        waitUntil: "domcontentloaded",
        timeout: 60000,
      });
      await installDomCore(runtime.page);
      return runtime.page;
    } catch (replacementError) {
      runtime.health.cdp += 1;
      runtime.lastError = stableErrorCode(replacementError);
      return recoverChrome(runtime);
    }
  }
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
    log(`model "${modelLabel}" selection failed (${stableErrorCode(err)}); using current`);
  }
}

// NOTE: keep this table in sync with `fileMime` in
// integrations/oracle/nyxid_oracle.user.js (same allowlist, separate runtime —
// the userscript can't import and the worker ships as one self-contained file).
function fileMime(name) {
  const ext = (name.split(".").pop() || "").toLowerCase();
  return (
    {
      pdf: "application/pdf",
      png: "image/png",
      jpg: "image/jpeg",
      jpeg: "image/jpeg",
      webp: "image/webp",
      gif: "image/gif",
      bmp: "image/bmp",
      svg: "image/svg+xml",
      txt: "text/plain",
      csv: "text/csv",
      md: "text/markdown",
      json: "application/json",
    }[ext] || "application/octet-stream"
  );
}

// Attach a general input file (image / pdf / text / ...) to the composer on the
// first turn so the model can answer questions about it. Mime is derived from
// the filename extension. Parallels uploadPdf; same file-input + attachment-chip
// detection. (uploadPdf is kept for the legacy pdf_base64 field.)
async function uploadAttachment(runtime, page, task) {
  if (!task.attachment_base64) return false;
  const buffer = Buffer.from(task.attachment_base64, "base64");
  const name = task.attachment_name || "attachment.bin";
  const mime = fileMime(name);
  log(`uploading attachment (${(buffer.length / 1024).toFixed(0)} KB, ${mime})`);
  let fileInput = page.locator("input[type='file']").first();
  if ((await fileInput.count()) === 0) {
    const attach = page.locator("button[aria-label='Attach files'], button[aria-label='Upload file'], button[data-testid='composer-attach-button'], button[aria-haspopup='menu']").first();
    if (await attach.count()) { await attach.click().catch(() => {}); await sleep(800); }
    fileInput = page.locator("input[type='file']").first();
  }
  try {
    await fileInput.setInputFiles({ name, mimeType: mime, buffer }, { timeout: 30000 });
  } catch (e) { log(`attachment upload failed (${stableErrorCode(e)})`); return false; }
  const start = Date.now();
  let lastHeartbeat = start;
  while (Date.now() - start < 120000) {
    await sleep(1500);
    if (Date.now() - lastHeartbeat >= HEARTBEAT_MS) {
      lastHeartbeat = Date.now();
      if (await ack(runtime, task, "uploading_attachment")) {
        throw new TaskFailure("cancelled");
      }
    }
    const { attached, uploading } = await page.evaluate((fname) => {
      const txt = document.body.innerText || "";
      return {
        attached: txt.includes(fname)
          || !!document.querySelector("[data-testid*='file'],[class*='file-chip'],[class*='attachment']"),
        uploading: !!document.querySelector("[role='progressbar'],[class*='uploading']"),
      };
    }, name);
    if (attached && !uploading) { log(`attachment attached (${Math.round((Date.now() - start) / 1000)}s)`); return true; }
  }
  log("attachment upload wait timed out — sending anyway");
  return false;
}

async function uploadPdf(runtime, page, task) {
  if (!task.pdf_base64) return false;
  const buffer = Buffer.from(task.pdf_base64, "base64");
  const name = task.pdf_name || "attachment.pdf";
  log(`uploading PDF (${(buffer.length / 1024).toFixed(0)} KB)`);
  let fileInput = page.locator("input[type='file']").first();
  if ((await fileInput.count()) === 0) {
    const attach = page.locator("button[aria-label='Attach files'], button[aria-label='Upload file'], button[data-testid='composer-attach-button'], button[aria-haspopup='menu']").first();
    if (await attach.count()) { await attach.click().catch(() => {}); await sleep(800); }
    fileInput = page.locator("input[type='file']").first();
  }
  try {
    await fileInput.setInputFiles({ name, mimeType: "application/pdf", buffer }, { timeout: 30000 });
  } catch (e) { log(`PDF upload failed (${stableErrorCode(e)})`); return false; }
  const start = Date.now();
  let lastHeartbeat = start;
  while (Date.now() - start < 120000) {
    await sleep(1500);
    // Keep the task lease warm + honor a server-side cancel during a long upload
    // (matches the heartbeat discipline in waitForResponse).
    if (Date.now() - lastHeartbeat >= HEARTBEAT_MS) {
      lastHeartbeat = Date.now();
      if (await ack(runtime, task, "uploading_pdf")) throw new TaskFailure("cancelled");
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

class TaskFailure extends Error {
  constructor(code) {
    super(code);
    this.code = code;
  }
}

class TaskRestart extends Error {}

function taskIdentity(runtime, task, extra = {}) {
  return {
    task_id: task.task_id,
    worker: LABEL,
    instance_id: runtime.state.instance_id,
    dispatch_attempt_id: task.dispatch_attempt_id,
    ...extra,
  };
}

async function pinCurrentConversation(runtime, page, task) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const url = page.url();
    if (convId(url)) {
      updateTaskState(runtime.state, { conversation_url: url });
      await apiPost("/pin-conv-url", taskIdentity(runtime, task, { chatgpt_url: url }));
      return url;
    }
    await sleep(500);
  }
  return null;
}

async function transcriptSnapshot(page) {
  return page.evaluate(() => {
    const main = document.querySelector("main");
    const composer = document.querySelector(
      "#prompt-textarea, div[contenteditable='true'][role='textbox'], textarea[data-testid='prompt-textarea']"
    );
    return {
      ready: Boolean(main && composer && window.__nyx),
      generating: Boolean(window.__nyx?.isStillGenerating()),
      turns: window.__nyx?.extractTranscript() || [],
      assistantCount: window.__nyx?.assistantCount() || 0,
      images: window.__nyx?.extractImages() || [],
    };
  });
}

async function submitPromptResult(runtime, page, task, text, imageSources = []) {
  updateTaskState(runtime.state, { phase: "settling", conversation_url: page.url() });
  const downloaded = await downloadImages(page, imageSources);
  const response = text || "";
  if (!response.trim() && downloaded.length === 0) {
    throw new TaskFailure("empty_extraction");
  }
  const result = await apiPost(
    "/result",
    taskIdentity(runtime, task, {
      response,
      images: downloaded,
      chatgpt_url: page.url(),
      model: task.model,
    })
  );
  log(
    `prompt ${task.task_id} -> ${result.status} (${response.length} chars, ${downloaded.length} image(s))`
  );
}

async function handlePrompt(runtime, page, task, recovering) {
  const { task_id } = task;
  log(`prompt task ${task_id} (followup=${!!task.is_followup})`);
  await page.bringToFront().catch(() => {});

  // Navigate: continue an existing conversation, or start a FRESH chat.
  // For a fresh prompt we must leave any /c/<uuid> page we're parked on,
  // otherwise we'd type into the previous conversation.
  const persistedUrl = runtime.state.current_task?.conversation_url;
  const priorPhase = runtime.state.current_task?.phase || "claimed";
  const navigation = choosePromptNavigation({
    recovering,
    phase: priorPhase,
    isFollowup: task.is_followup,
    currentUrl: page.url(),
    persistedUrl,
    taskConversationUrl: task.conversation_url,
    requiredProjectUrl: task.required_project_url,
  });
  if (navigation.error) throw new TaskFailure(navigation.error);
  const navTarget = navigation.target;
  if (navTarget) {
    await page.goto(navTarget, { waitUntil: "domcontentloaded" });
    await installDomCore(page);
    await page.bringToFront().catch(() => {});
    await sleep(2500);
  }

  updateTaskState(runtime.state, {
    phase: ["claimed", "page_ready", "ready_to_send"].includes(priorPhase)
      ? "page_ready"
      : priorPhase,
    conversation_url: page.url(),
  });
  await ack(runtime, task, "page_ready");

  if (recovering && !["claimed", "page_ready", "ready_to_send"].includes(priorPhase)) {
    await sleep(1500);
    const snapshot = await transcriptSnapshot(page);
    const decision = decidePromptResume({
      phase: priorPhase,
      prompt: task.prompt,
      turns: snapshot.turns,
      generating: snapshot.generating,
      transcriptReady: snapshot.ready,
      baselineTurnCount: runtime.state.current_task?.baseline_turn_count || 0,
    });
    if (decision.action === "complete") {
      await submitPromptResult(runtime, page, task, decision.response, snapshot.images);
      return;
    }
    if (decision.action === "wait") {
      const beforeCount = snapshot.turns
        .slice(0, runtime.state.current_task?.baseline_turn_count || 0)
        .filter((turn) => turn.role === "assistant").length;
      const answer = await waitForResponse(runtime, page, task, beforeCount);
      await submitPromptResult(runtime, page, task, answer.text, answer.images);
      return;
    }
    if (decision.action === "uncertain") {
      throw new TaskFailure("prompt_delivery_uncertain");
    }
  }

  if (task.model && task.model !== "unknown") {
    await ack(runtime, task, "selecting_model");
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
  const baseline = await page.evaluate(() => window.__nyx?.extractTranscript()?.length || 0);
  updateTaskState(runtime.state, { phase: "ready_to_send", baseline_turn_count: baseline });
  await sleep(300);
  // Only attach a PDF on the FIRST turn of a conversation — never re-upload it
  // into an existing chat if the server ever resends pdf_base64 on a follow-up
  // (mirrors the userscript's `!is_followup && pdf_base64` guard).
  if (!task.is_followup && task.pdf_base64) {
    await ack(runtime, task, "uploading_pdf");
    await uploadPdf(runtime, page, task);
  }
  // Same first-turn-only guard for a general attachment (image / pdf / ...).
  if (!task.is_followup && task.attachment_base64) {
    await ack(runtime, task, "uploading_attachment");
    await uploadAttachment(runtime, page, task);
  }

  const beforeCount = await page.evaluate(() => window.__nyx.assistantCount());
  const sendBtn = page
    .locator("button[data-testid='send-button'], button[aria-label='Send prompt'], button[aria-label='发送提示']")
    .first();
  updateTaskState(runtime.state, { phase: "send_attempted", baseline_turn_count: baseline });
  await sendBtn.click({ timeout: 30000 });
  updateTaskState(runtime.state, { phase: "sent" });
  await ack(runtime, task, "sent");
  await pinCurrentConversation(runtime, page, task);

  const { text, images } = await waitForResponse(runtime, page, task, beforeCount);
  await submitPromptResult(runtime, page, task, text, images);
}

function convId(url) {
  const m = (url || "").match(/\/c\/([a-f0-9-]{6,})/);
  return m ? m[1] : null;
}

// Returns { text, images } where images is an array of on-page src strings
// for the latest assistant turn. An image-generation turn settles with empty
// text but a non-empty images list; the stability key spans both so the turn
// terminates once text AND image srcs stop changing.
async function waitForResponse(runtime, page, task, beforeCount) {
  const start = Date.now();
  let lastHeartbeat = start;
  let lastKey = "";
  let stable = 0;
  while (Date.now() - start < MAX_WAIT_MS) {
    await sleep(STABLE_INTERVAL_MS);
    if (Date.now() - lastHeartbeat >= HEARTBEAT_MS) {
      lastHeartbeat = Date.now();
      updateTaskState(runtime.state, { phase: "waiting_response", conversation_url: page.url() });
      const cancelled = await ack(runtime, task, "waiting_response");
      if (cancelled) throw new TaskFailure("cancelled");
      await heartbeat(runtime);
      if (
        runtime.state.pending_command?.command === "session_import" &&
        runtime.loggedIn === false
      ) {
        await processPendingCommand(runtime, true);
        throw new TaskRestart();
      }
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
          log(`image fetch returned HTTP ${resp.status()}`);
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
      log(`image download failed (${stableErrorCode(e)})`);
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

async function handleScrape(runtime, page, task) {
  const { task_id, conversation_url } = task;
  log(`scrape task ${task_id}`);
  await page.bringToFront().catch(() => {});
  if (!conversation_url) {
    throw new TaskFailure("conversation_url_missing");
  }
  await page.goto(conversation_url, { waitUntil: "domcontentloaded" });
  await installDomCore(page);
  await page.bringToFront().catch(() => {});
  updateTaskState(runtime.state, { phase: "scraping", conversation_url: page.url() });
  await ack(runtime, task, "scraping");

  const turns = await loadFullTranscript(page);
  const res = await apiPost(
    "/transcript",
    taskIdentity(runtime, task, { turns, chatgpt_url: page.url() })
  );
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

async function handleExtract(runtime, page, task) {
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
    updateTaskState(runtime.state, { phase: "extracting", conversation_url: page.url() });
    await ack(runtime, task, "extracting");
    await scrollLazyPage(page);
    await expandCollapsibles(page);
    const content = await page.evaluate(() => {
      const root = document.querySelector("main, article") || document.body;
      return ((root && root.innerText) || "").trim().slice(0, 200000);
    });
    const response = content || "ERROR: empty extraction";
    const res = await apiPost("/result", taskIdentity(runtime, task, {
      response,
      chatgpt_url: page.url(),
      model: task.model,
    }));
    log(`extract ${task_id} → ${res.status} (${content.length} chars)`);
  } catch (err) {
    await apiPost("/result", taskIdentity(runtime, task, {
      response: `ERROR: ${stableErrorCode(err)}`,
      chatgpt_url: page.url(),
      model: task.model,
    }));
  }
}

async function ack(runtime, task, phase) {
  const response = await apiPost("/ack", taskIdentity(runtime, task, {
    phase,
    page_url: runtime.page?.url(),
  }));
  return response.status === "cancelled";
}

function addCommandReport(state, command, succeeded, resultCode) {
  const report = {
    command_id: command.id,
    succeeded,
    result_code: resultCode,
  };
  state.pending_reports = [
    ...(state.pending_reports || []).filter((item) => item.command_id !== command.id),
    report,
  ].slice(-16);
  state.command_results = [
    ...(state.command_results || []).filter((item) => item.command_id !== command.id),
    report,
  ].slice(-64);
  state.pending_command = null;
  saveState(state);
}

function acceptCommand(runtime, command) {
  if (!command?.id || !command.command) return;
  const completed = (runtime.state.command_results || []).find(
    (item) => item.command_id === command.id
  );
  if (completed) {
    runtime.state.pending_reports = [
      ...(runtime.state.pending_reports || []).filter(
        (item) => item.command_id !== command.id
      ),
      completed,
    ].slice(-16);
    saveState(runtime.state);
    return;
  }
  if (runtime.state.pending_command?.id === command.id) return;
  if (command.command === "drain" || command.command === "resume") {
    runtime.state.draining = command.command === "drain";
    addCommandReport(
      runtime.state,
      command,
      true,
      command.command === "drain" ? "draining" : "resumed"
    );
    return;
  }
  runtime.state.draining = true;
  runtime.state.pending_command = command;
  saveState(runtime.state);
}

async function heartbeat(runtime) {
  let loggedIn = null;
  if (runtime.chromeAlive && runtime.page && !runtime.page.isClosed()) {
    loggedIn = await detectLoggedIn(runtime.page);
  }
  runtime.loggedIn = loggedIn;
  const reports = [...(runtime.state.pending_reports || [])].slice(0, 16);
  const response = await apiPost("/heartbeat", {
    worker: LABEL,
    instance_id: runtime.state.instance_id,
    platform: `${process.platform}-${process.arch}`,
    capabilities: CAPABILITIES,
    logged_in: loggedIn,
    current_task_id: runtime.state.current_task?.task_id || null,
    chrome_alive: runtime.chromeAlive,
    last_error: runtime.lastError || null,
    command_reports: reports,
  });
  if (reports.length) {
    const sent = new Set(reports.map((report) => report.command_id));
    runtime.state.pending_reports = (runtime.state.pending_reports || []).filter(
      (report) => !sent.has(report.command_id)
    );
    saveState(runtime.state);
  }
  acceptCommand(runtime, response.command);
  runtime.lastPresenceAt = Date.now();
  return response;
}

export function decryptSessionEnvelope(sealedBytes, token) {
  if (!Buffer.isBuffer(sealedBytes)) sealedBytes = Buffer.from(sealedBytes);
  if (!sealedBytes.length || sealedBytes.length > MAX_SESSION_SNAPSHOT_BYTES) {
    throw new TaskFailure("session_envelope_size_invalid");
  }
  let envelope;
  try {
    envelope = JSON.parse(sealedBytes.toString("utf8"));
  } catch {
    throw new TaskFailure("session_envelope_invalid");
  }
  if (envelope?.version !== SESSION_FORMAT_VERSION) {
    throw new TaskFailure("session_envelope_version_unsupported");
  }
  const salt = Buffer.from(envelope.salt_base64 || "", "base64");
  const nonce = Buffer.from(envelope.nonce_base64 || "", "base64");
  const ciphertext = Buffer.from(envelope.ciphertext_base64 || "", "base64");
  if (salt.length !== 32 || nonce.length !== 12 || ciphertext.length < 16) {
    throw new TaskFailure("session_envelope_invalid");
  }
  try {
    const key = Buffer.from(hkdfSync("sha256", Buffer.from(token), salt, SESSION_INFO, 32));
    const body = ciphertext.subarray(0, ciphertext.length - 16);
    const tag = ciphertext.subarray(ciphertext.length - 16);
    const decipher = createDecipheriv("aes-256-gcm", key, nonce);
    decipher.setAAD(SESSION_AAD);
    decipher.setAuthTag(tag);
    const plaintext = Buffer.concat([decipher.update(body), decipher.final()]);
    if (plaintext.length > MAX_SESSION_PLAINTEXT_BYTES) {
      throw new TaskFailure("session_plaintext_too_large");
    }
    return JSON.parse(plaintext.toString("utf8"));
  } catch (error) {
    if (error instanceof TaskFailure) throw error;
    throw new TaskFailure("session_decrypt_failed");
  }
}

function allowedSessionCookie(cookie) {
  const domain = String(cookie?.domain || "").replace(/^\./, "").toLowerCase();
  return (
    (domain === "chatgpt.com" || domain.endsWith(".chatgpt.com") ||
      domain === "openai.com" || domain.endsWith(".openai.com")) &&
    typeof cookie.name === "string" &&
    typeof cookie.value === "string" &&
    cookie.name.length <= 256 &&
    cookie.value.length <= 16384
  );
}

function allowedSessionOrigin(origin) {
  return ["https://chatgpt.com", "https://auth.openai.com"].includes(origin);
}

async function importLoginSnapshot(runtime, command) {
  const payload = await apiGet(`/login-snapshots/${encodeURIComponent(command.snapshot_id || "")}`);
  if (payload.format_version !== SESSION_FORMAT_VERSION) {
    throw new TaskFailure("session_snapshot_version_unsupported");
  }
  const snapshot = decryptSessionEnvelope(
    Buffer.from(payload.sealed_blob_base64 || "", "base64"),
    TOKEN
  );
  if (snapshot?.version !== SESSION_FORMAT_VERSION || !Array.isArray(snapshot.cookies)) {
    throw new TaskFailure("session_snapshot_invalid");
  }
  await ensureChatPage(runtime);
  await runtime.context.clearCookies({ domain: /(^|\.)chatgpt\.com$/ }).catch(() => {});
  await runtime.context.clearCookies({ domain: /(^|\.)openai\.com$/ }).catch(() => {});
  const cookies = snapshot.cookies.filter(allowedSessionCookie).slice(0, 512);
  if (!cookies.length) throw new TaskFailure("session_snapshot_no_cookies");
  await runtime.context.addCookies(cookies);
  for (const storage of (snapshot.origins || []).filter((entry) => allowedSessionOrigin(entry?.origin))) {
    await runtime.page.goto(storage.origin, { waitUntil: "domcontentloaded", timeout: 60000 });
    if (new URL(runtime.page.url()).origin !== storage.origin) continue;
    await runtime.page.evaluate((entry) => {
      for (const item of entry.local_storage || []) {
        if (typeof item.name === "string" && typeof item.value === "string") {
          localStorage.setItem(item.name, item.value);
        }
      }
      for (const item of entry.session_storage || []) {
        if (typeof item.name === "string" && typeof item.value === "string") {
          sessionStorage.setItem(item.name, item.value);
        }
      }
    }, storage);
  }
  await runtime.page.goto("https://chatgpt.com/", {
    waitUntil: "domcontentloaded",
    timeout: 60000,
  });
  await installDomCore(runtime.page);
  for (let attempt = 0; attempt < 30; attempt += 1) {
    if (await detectLoggedIn(runtime.page)) return "session_import_verified";
    await sleep(2000);
  }
  throw new TaskFailure("session_import_verification_failed");
}

async function installBundle(command) {
  const response = await apiGet("/bundle");
  const source = response.bundle || "";
  const actual = createHash("sha256").update(source).digest("hex");
  const version = String(response.version || "");
  const playwrightVersion = String(response.playwright_core_version || "1.62.1");
  if (
    !/^[a-f0-9]{64}$/.test(response.sha256 || "") ||
    response.sha256 !== actual ||
    (command.bundle_sha256 && command.bundle_sha256 !== actual) ||
    !/^[A-Za-z0-9._+-]{1,128}$/.test(version) ||
    !version.endsWith(actual.slice(0, 12)) ||
    (command.bundle_version && command.bundle_version !== version) ||
    !/^\d+\.\d+\.\d+$/.test(playwrightVersion)
  ) {
    throw new TaskFailure("bundle_checksum_mismatch");
  }
  const target = resolve(process.argv[1]);
  const installDir = dirname(target);
  const packagePath = resolve(installDir, "package.json");
  const packageTemp = `${packagePath}.upgrade-${process.pid}`;
  const packageBody = {
    name: "nyxid-oracle-worker-install",
    private: true,
    type: "module",
    dependencies: { "playwright-core": playwrightVersion },
    nyxid_bundle_version: version,
  };
  writeFileSync(packageTemp, `${JSON.stringify(packageBody, null, 2)}\n`, { mode: 0o600 });
  chmodSync(packageTemp, 0o600);
  renameSync(packageTemp, packagePath);
  await new Promise((resolveInstall, rejectInstall) => {
    const child = spawn(
      NPM_EXECUTABLE,
      ["install", "--omit=dev", "--no-audit", "--no-fund", "--save-exact"],
      { cwd: installDir, stdio: "inherit" }
    );
    const timeout = setTimeout(() => {
      child.kill("SIGTERM");
      rejectInstall(Object.assign(new Error("npm install timed out"), {
        code: "upgrade_dependency_install_timeout",
      }));
    }, NPM_INSTALL_TIMEOUT_MS);
    child.once("error", (error) => {
      clearTimeout(timeout);
      rejectInstall(error);
    });
    child.once("exit", (code, signal) => {
      clearTimeout(timeout);
      if (code === 0) resolveInstall();
      else rejectInstall(Object.assign(new Error(`npm install failed (${code ?? signal})`), {
        code: "upgrade_dependency_install_failed",
      }));
    });
  });
  const mode = statSync(target).mode & 0o777;
  const temp = `${target}.upgrade-${process.pid}`;
  const versionTemp = `${BUNDLE_VERSION_FILE}.upgrade-${process.pid}`;
  writeFileSync(temp, source, { mode });
  chmodSync(temp, mode);
  writeFileSync(versionTemp, `${version}\n`, { mode: 0o644 });
  chmodSync(versionTemp, 0o644);
  renameSync(temp, target);
  renameSync(versionTemp, BUNDLE_VERSION_FILE);
  return "upgrade_installed";
}

async function processPendingCommand(runtime, allowSessionImportDuringLoggedOutTask = false) {
  const command = runtime.state.pending_command;
  if (
    !command ||
    (runtime.state.current_task &&
      !(allowSessionImportDuringLoggedOutTask && command.command === "session_import"))
  ) {
    return false;
  }
  try {
    let resultCode;
    let shouldExit = false;
    switch (command.command) {
      case "restart":
        resultCode = "restarting";
        shouldExit = true;
        break;
      case "relaunch_browser":
        await recoverChrome(runtime, true);
        resultCode = "browser_relaunched";
        break;
      case "relogin":
        await ensureChatPage(runtime, "https://chatgpt.com/auth/login");
        resultCode = "login_page_opened";
        break;
      case "session_import":
        resultCode = await importLoginSnapshot(runtime, command);
        break;
      case "upgrade":
        resultCode = await installBundle(command);
        shouldExit = true;
        break;
      default:
        throw new TaskFailure("command_unsupported");
    }
    runtime.state.draining = command.command === "restart" || command.command === "upgrade";
    runtime.lastError = null;
    addCommandReport(runtime.state, command, true, resultCode);
    await heartbeat(runtime);
    if (shouldExit) {
      runtime.state.draining = false;
      saveState(runtime.state);
    }
    return shouldExit;
  } catch (error) {
    const code = stableErrorCode(error);
    runtime.lastError = code;
    runtime.state.draining = false;
    addCommandReport(runtime.state, command, false, code);
    await heartbeat(runtime);
    return false;
  }
}

async function settleTaskFailure(runtime, task, code) {
  if (code === "cancelled") return;
  await apiPost(
    "/result",
    taskIdentity(runtime, task, {
      response: `ERROR: ${code}`,
      chatgpt_url: runtime.page?.url(),
      model: task.model,
    })
  );
}

async function executeTask(runtime, task, recovering) {
  for (;;) {
    try {
      const resumeUrl =
        runtime.state.current_task?.conversation_url || task.conversation_url || undefined;
      const page = await ensureChatPage(runtime, resumeUrl);
      if (task.kind === "scrape") await handleScrape(runtime, page, task);
      else if (task.kind === "extract") await handleExtract(runtime, page, task);
      else await handlePrompt(runtime, page, task, recovering);
      clearTaskState(runtime.state);
      runtime.lastError = null;
      return;
    } catch (error) {
      if (error instanceof TaskRestart) {
        recovering = true;
        continue;
      }
      if (error instanceof TaskFailure) {
        const code = stableErrorCode(error);
        runtime.lastError = code === "cancelled" ? null : code;
        await settleTaskFailure(runtime, task, code);
        clearTaskState(runtime.state);
        return;
      }
      const failureCount = (runtime.state.current_task?.recovery_failures || 0) + 1;
      updateTaskState(runtime.state, { recovery_failures: failureCount });
      const recovery = taskRecoveryDecision({
        kind: task.kind,
        phase: runtime.state.current_task?.phase,
        failureCount,
      });
      runtime.chromeAlive = false;
      runtime.lastError = stableErrorCode(error);
      if (recovery.action === "fail") {
        runtime.lastError = recovery.code;
        await settleTaskFailure(runtime, task, recovery.code);
        clearTaskState(runtime.state);
        return;
      }
      runtime.health.cdp += 1;
      log(`task ${task.task_id} paused for browser recovery (${runtime.lastError})`);
      await recoverChrome(runtime, recovery.forceRelaunch);
      recovering = true;
    }
  }
}

async function captureStorage(page) {
  const origin = new URL(page.url()).origin;
  if (!allowedSessionOrigin(origin)) return null;
  return page.evaluate(() => ({
    origin: location.origin,
    local_storage: Object.keys(localStorage).map((name) => ({ name, value: localStorage.getItem(name) || "" })),
    session_storage: Object.keys(sessionStorage).map((name) => ({ name, value: sessionStorage.getItem(name) || "" })),
  }));
}

async function captureSession(outputPath) {
  const browser = await chromium.connectOverCDP(CDP_URL);
  try {
    const context = browser.contexts()[0] || (await browser.newContext());
    const page = await getChatPage(context);
    const timeoutMs = Number(process.env.NYXID_LOGIN_CAPTURE_TIMEOUT_MS || 15 * 60 * 1000);
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline && !(await detectLoggedIn(page))) await sleep(1000);
    if (!(await detectLoggedIn(page))) throw new TaskFailure("login_capture_timeout");
    const cookies = (await context.cookies([
      "https://chatgpt.com/",
      "https://auth.openai.com/",
    ])).filter(allowedSessionCookie);
    const origins = [];
    for (const candidate of context.pages()) {
      const storage = await captureStorage(candidate).catch(() => null);
      if (storage && !origins.some((item) => item.origin === storage.origin)) origins.push(storage);
    }
    const snapshot = Buffer.from(
      JSON.stringify({ version: SESSION_FORMAT_VERSION, captured_at: new Date().toISOString(), cookies, origins }),
      "utf8"
    );
    if (!cookies.length || snapshot.length > MAX_SESSION_PLAINTEXT_BYTES) {
      throw new TaskFailure(!cookies.length ? "login_capture_no_cookies" : "login_capture_too_large");
    }
    mkdirSync(dirname(outputPath), { recursive: true, mode: 0o700 });
    writeFileSync(outputPath, snapshot, { mode: 0o600 });
    chmodSync(outputPath, 0o600);
  } finally {
    await browser.close().catch(() => {});
  }
}

// ── Main loop ────────────────────────────────────────────────────────────
async function main() {
  const captureIndex = process.argv.indexOf("--capture-session");
  if (captureIndex >= 0) {
    const output = process.argv[captureIndex + 1];
    if (!output) throw new Error("--capture-session requires an output path");
    await captureSession(resolve(output));
    return;
  }
  if (!BASE_URL || !TOKEN) {
    throw new Error(
      "Set NYXID_BASE_URL and NYXID_WORKER_TOKEN_FILE (preferred) or NYXID_WORKER_TOKEN"
    );
  }

  const state = loadState();
  saveState(state);
  const runtime = {
    state,
    browser: null,
    context: null,
    page: null,
    chromeAlive: false,
    lastError: null,
    loggedIn: null,
    lastPresenceAt: 0,
    health: { http: 0, cdp: 0, tab: 0 },
  };
  log(`starting worker=${LABEL} version=${SCRIPT_VERSION}`);
  await recoverChrome(runtime);

  for (;;) {
    try {
      if (Date.now() - runtime.lastPresenceAt >= PRESENCE_MS) await heartbeat(runtime);
      if (await processPendingCommand(runtime)) process.exit(75);
      if (runtime.state.draining && !runtime.state.current_task) {
        await sleep(POLL_MS);
        continue;
      }
      const page = await ensureChatPage(runtime);
      const response = await apiGet(
        `/task?worker=${encodeURIComponent(LABEL)}` +
          `&script_version=${encodeURIComponent(SCRIPT_VERSION)}` +
          `&instance_id=${encodeURIComponent(state.instance_id)}` +
          `&page_url=${encodeURIComponent(page.url())}`
      );
      if (response.status === "idle") {
        if (state.current_task) clearTaskState(state);
        if (
          response.required_project_url &&
          !page.url().startsWith(response.required_project_url)
        ) {
          await ensureChatPage(runtime, response.required_project_url);
        }
      } else if (response.status === "task" && response.task_id) {
        const recovering = state.current_task?.task_id === response.task_id;
        if (!recovering) {
          state.current_task = {
            task_id: response.task_id,
            dispatch_attempt_id: response.dispatch_attempt_id || null,
            conversation_url: response.conversation_url || null,
            phase: "claimed",
            baseline_turn_count: 0,
          };
          saveState(state);
        } else {
          updateTaskState(state, {
            dispatch_attempt_id: response.dispatch_attempt_id || null,
            conversation_url:
              state.current_task.conversation_url || response.conversation_url || null,
          });
        }
        await executeTask(runtime, response, recovering);
      }
    } catch (error) {
      runtime.lastError = stableErrorCode(error);
      if (error?.status === 401 || error?.status === 403) {
        log(`worker authentication rejected (HTTP ${error.status}); retrying after 30s`);
        await sleep(30000);
      } else {
        log(`worker loop paused (${runtime.lastError})`);
        await sleep(backoffDelay(1, POLL_MS, 30000));
      }
    }
    await sleep(POLL_MS);
  }
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  main().catch((error) => {
    console.error(`fatal: ${stableErrorCode(error)}`);
    process.exit(1);
  });
}
