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
  createCipheriv,
  createDecipheriv,
  createHash,
  createHmac,
  hkdfSync,
  randomUUID,
  randomBytes,
} from "node:crypto";
import { lookup } from "node:dns/promises";
import {
  chmodSync,
  mkdirSync,
  readFileSync,
  renameSync,
  statSync,
  writeFileSync, realpathSync, existsSync } from "node:fs";
import { isIP } from "node:net";
import { homedir } from "node:os";
import { dirname, resolve } from "node:path";
import { execFileSync, spawn } from "node:child_process";
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
// The daemon runs under launchd/systemd with a minimal PATH, so bare "npm"
// (and npm's own `#!/usr/bin/env node` shebang) can fail with ENOENT. Prefer
// an explicitly configured npm, then the npm that ships beside this node
// binary, then PATH lookup with node's directory prepended.
export function resolveNpmExecutable({ configured, execPath, exists = existsSync } = {}) {
  if (configured) return configured;
  const sibling = resolve(dirname(execPath || process.execPath), "npm");
  if (exists(sibling)) return sibling;
  return "npm";
}
export function daemonPath({ execPath, envPath } = {}) {
  const nodeDir = dirname(execPath || process.execPath);
  const base = envPath || "/usr/local/bin:/usr/bin:/bin";
  return base.split(":").includes(nodeDir) ? base : `${nodeDir}:${base}`;
}
export function installedDependencyVersion(installDir, read = readFileSync) {
  try {
    return JSON.parse(read(resolve(installDir, "node_modules/playwright-core/package.json"), "utf8")).version || null;
  } catch {
    return null;
  }
}
const NPM_EXECUTABLE = resolveNpmExecutable({ configured: process.env.NYXID_NPM_EXECUTABLE });
const NPM_INSTALL_TIMEOUT_MS = Number(
  process.env.NYXID_NPM_INSTALL_TIMEOUT_MS || 5 * 60 * 1000
);
export function workerCapabilities(token) {
  const capabilities = ["commands_v1", "upgrade_v1", "attempt_fencing_v1"];
  if (!String(token).startsWith("nyx_owi_")) capabilities.push("session_import_v1", "saved_login_v1");
  return capabilities;
}
const CAPABILITIES = workerCapabilities(TOKEN);
const SAVED_LOGIN_POLL_MS = Number(process.env.NYXID_SAVED_LOGIN_POLL_MS || 60000);
const SAVED_LOGIN_REFRESH_MS = Number(process.env.NYXID_SAVED_LOGIN_REFRESH_MS || 300000);
// Result-image caps (the server re-validates and caps lower-or-equal). Kept
// below the 16 MiB worker body cap once base64-inflated (~33%).
const MAX_IMAGES = Math.min(Number(process.env.NYXID_MAX_IMAGES || 4), 8);
const MAX_IMAGE_BYTES = Math.min(
  Number(process.env.NYXID_MAX_IMAGE_BYTES || 6 * 1024 * 1024),
  8_000_000
);
const MAX_IMAGES_TOTAL_BYTES = Math.min(
  Number(process.env.NYXID_MAX_IMAGES_TOTAL_BYTES || 9 * 1024 * 1024),
  9 * 1024 * 1024
);
const MAX_FILES = Math.min(Number(process.env.NYXID_MAX_FILES || 8), 8);
const MAX_FILE_BYTES = Math.min(
  Number(process.env.NYXID_MAX_FILE_BYTES || 6 * 1024 * 1024),
  8_000_000
);
const MAX_FILES_TOTAL_BYTES = Math.min(
  Number(process.env.NYXID_MAX_FILES_TOTAL_BYTES || 9 * 1024 * 1024),
  9 * 1024 * 1024
);
// Nine decoded MiB base64-inflate to twelve MiB, leaving room for response
// text and JSON framing under the worker route's 16 MiB request limit.
const MAX_ARTIFACTS_TOTAL_BYTES = Math.min(
  Number(process.env.NYXID_MAX_ARTIFACTS_TOTAL_BYTES || 9 * 1024 * 1024),
  9 * 1024 * 1024
);

export function artifactFileId(value) {
  const match = String(value || "").match(/file[-_][A-Za-z0-9]+/i);
  return match ? match[0].toLowerCase() : null;
}

export function isTrustedArtifactUrl(value) {
  try {
    const raw = String(value || "");
    if (raw.startsWith("blob:")) {
      const inner = new URL(raw.slice("blob:".length));
      return (
        inner.protocol === "https:" &&
        (inner.hostname === "chatgpt.com" || inner.hostname === "chat.openai.com")
      );
    }
    const url = new URL(raw);
    if (url.protocol !== "https:") return false;
    const host = url.hostname.toLowerCase();
    if (host === "oaiusercontent.com" || host.endsWith(".oaiusercontent.com")) {
      return true;
    }
    return (
      (host === "chatgpt.com" || host === "chat.openai.com") &&
      url.pathname.startsWith("/backend-api/")
    );
  } catch {
    return false;
  }
}

export function sanitizeArtifactName(value, fallbackIndex = 1) {
  const chars = Array.from(String(value || "").trim()).slice(0, 128);
  let safe = chars
    .map((char) =>
      char === "/" ||
      char === "\\" ||
      /[<>:"|?*]/.test(char) ||
      char.charCodeAt(0) < 32 ||
      char.charCodeAt(0) === 127
        ? "_"
        : char
    )
    .join("")
    .replace(/^\.+/, "")
    .replace(/[. ]+$/, "")
    .trim();
  if (!safe) safe = `file_${fallbackIndex}`;
  return safe;
}

export function classifyArtifactLink(link, imageSources = [], fallbackIndex = 1) {
  const href = String(link?.href || "");
  if (!isTrustedArtifactUrl(href)) return null;
  const key = artifactFileId(href) || href;
  const imageKeys = new Set(
    (imageSources || []).map((src) => artifactFileId(src) || String(src || ""))
  );
  if (imageKeys.has(key)) return null;

  let urlName = "";
  try {
    const url = href.startsWith("blob:")
      ? new URL(href.slice("blob:".length))
      : new URL(href);
    const segment = url.pathname.split("/").filter(Boolean).pop() || "";
    urlName =
      url.searchParams.get("filename") ||
      url.searchParams.get("name") ||
      (segment === "content" ? artifactFileId(href) || "" : segment);
    try {
      urlName = decodeURIComponent(urlName);
    } catch {}
  } catch {}
  const name = sanitizeArtifactName(
    link?.download || link?.text || urlName,
    fallbackIndex
  );
  return { href, name, key };
}

export function artifactBudgetDecision(used, size, perItemLimit, totalLimit) {
  if (!Number.isSafeInteger(size) || size <= 0 || size > perItemLimit) return "skip";
  if (used + size > totalLimit) return "stop";
  return "accept";
}

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
    drain_requested: false,
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
  if (error?.code && /^[A-Z][A-Z0-9_]{1,63}$/.test(error.code)) return error.code.toLowerCase();
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

async function apiRequest(method, path, body, retry = true) {
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
      if (!retry) throw error;
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
  const artifactFileId = ${artifactFileId.toString()};
  const isTrustedArtifactUrl = ${isTrustedArtifactUrl.toString()};
  const sanitizeArtifactName = ${sanitizeArtifactName.toString()};
  const classifyArtifactLink = ${classifyArtifactLink.toString()};

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

  function latestAssistantTurn() {
    const main = document.querySelector("main");
    if (!main) return null;
    const turns = main.querySelectorAll('[data-testid^="conversation-turn"]');
    if (turns.length) {
      const scope = turns[turns.length - 1];
      return scope.querySelector("[data-message-author-role='user']") ? null : scope;
    }
    const els = main.querySelectorAll("[data-message-author-role='assistant']");
    return els.length ? els[els.length - 1] : null;
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
    const scope = latestAssistantTurn();
    if (!scope) return [];
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

  // Download links in the latest assistant turn. Link hrefs are untrusted
  // model output: classification admits only ChatGPT content endpoints, and
  // downloadFiles repeats that check immediately before every fetch.
  function extractFiles() {
    const scope = latestAssistantTurn();
    if (!scope) return [];
    const images = extractImages();
    const out = [];
    const seen = new Set();
    for (const anchor of Array.from(scope.querySelectorAll("a[href]"))) {
      const file = classifyArtifactLink(
        {
          href: anchor.href || anchor.getAttribute("href") || "",
          download: anchor.getAttribute("download") || "",
          text: (anchor.innerText || anchor.textContent || "").trim(),
        },
        images,
        out.length + 1
      );
      if (!file || seen.has(file.key)) continue;
      seen.add(file.key);
      out.push({ href: file.href, name: file.name });
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

  // A picker interaction owns only menus newly visible after its trigger.
  // Element identity matters: a sidebar listbox can outlive every task, and
  // a picker can reuse a previously hidden menu node without changing counts.
  let modelPickerId = null;
  let preexistingModelMenus = new WeakSet();
  function pickerElementVisible(el) {
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none";
  }
  function visibleModelMenus() {
    return [...document.querySelectorAll('[role="menu"], [role="listbox"]')].filter(pickerElementVisible);
  }
  function beginModelPicker(id) {
    modelPickerId = id;
    preexistingModelMenus = new WeakSet(visibleModelMenus());
    return true;
  }
  function modelPickerMenus(id) {
    if (!id || id !== modelPickerId) return [];
    return visibleModelMenus().filter((menu) => !preexistingModelMenus.has(menu));
  }
  function modelPickerItems(id) {
    const menus = modelPickerMenus(id);
    return [...document.querySelectorAll('[role="menuitemradio"], [role="menuitem"], [role="option"]')]
      .filter((el) => pickerElementVisible(el) && menus.includes(el.closest('[role="menu"], [role="listbox"]')))
      .slice(0, 64);
  }
  function modelPickerTrigger(id) {
    return modelPickerMenus(id).flatMap((menu) => [...menu.querySelectorAll('[data-testid="composer-intelligence-pro-thinking-effort-trigger"]')])
      .find((el) => pickerElementVisible(el) && modelPickerMenus(id).includes(el.closest('[role="menu"], [role="listbox"]'))) || null;
  }
  function modelPickerItem(id, index, text) {
    const item = modelPickerItems(id)[index];
    return item && (item.innerText || item.textContent || "").trim() === text ? item : null;
  }

  return { isStillGenerating, assistantCount, extractResponse, extractImages, extractFiles, extractTranscript, extractTranscriptKeys, scrollContainer, extractTextWithMath, cleanText,
    beginModelPicker, modelPickerMenus, modelPickerItems, modelPickerTrigger, modelPickerItem };
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

// Hosts a human passes through while logging into ChatGPT (password, OTP,
// SSO, Cloudflare). While the tab is on one of these the worker must not
// steer it, or the login can never complete.
const AUTH_FLOW_HOSTS =
  /^(auth\.openai\.com|auth0\.openai\.com|accounts\.google\.com|login\.microsoftonline\.com|login\.live\.com|appleid\.apple\.com|challenges\.cloudflare\.com)$/i;

export function isAuthFlowUrl(u) {
  let url;
  try {
    url = new URL(u || "");
  } catch {
    return false;
  }
  if (AUTH_FLOW_HOSTS.test(url.hostname)) return true;
  return /^(chatgpt\.com|chat\.openai\.com)$/i.test(url.hostname) && /^\/auth(\/|$)/.test(url.pathname);
}

export function isAccountChangeUrl(value, navigation = false) {
  let url;
  try { url = new URL(value); } catch { return false; }
  if (navigation && isAuthFlowUrl(value)) return true;
  if (!/^(chatgpt\.com|chat\.openai\.com|auth\.openai\.com|auth0\.openai\.com)$/i.test(url.hostname)) return false;
  return /(?:^|\/)(?:login|logout|signin|signout|switch-account|switch_account)(?:\/|$)/i.test(url.pathname);
}

export function invalidateSavedLogin(state, status) {
  const local = state.saved_login;
  if (!local || (local.status !== "verified" && !(local.status === "untrusted" && status === "external_login"))) return false;
  local.status = status;
  local.pending_publication_id = null;
  return true;
}

async function observeSavedLoginTrust(runtime, loggedIn) {
  if (runtime.applyingLogin || loggedIn !== false || !isChatGptUrl(runtime.page?.url())) return;
  const loggedOut = await runtime.page.evaluate(() => Array.from(document.querySelectorAll("a,button"))
    .some((element) => /^(log in|sign up|登录|注册)$/i.test((element.textContent || "").trim()))).catch(() => false);
  if (loggedOut && invalidateSavedLogin(runtime.state, "untrusted")) saveState(runtime.state);
}

function watchLoginChanges(runtime) {
  const context = runtime.context;
  const watch = (page) => {
    const changed = (url, navigation) => {
      if (!runtime.applyingLogin && isAccountChangeUrl(url, navigation) &&
          invalidateSavedLogin(runtime.state, "external_login")) saveState(runtime.state);
    };
    page.on("framenavigated", (frame) => {
      if (frame === page.mainFrame()) changed(frame.url(), true);
    });
    page.on("request", (request) => {
      try {
        if (request.frame() === page.mainFrame()) changed(request.url(), request.isNavigationRequest());
      } catch {}
    });
    changed(page.url(), true);
  };
  context.pages().forEach(watch);
  context.on("page", watch);
}

// Decide whether the worker keeps its hands off the tab: a live page that is
// mid-login, or a ChatGPT page the last heartbeat saw logged out, belongs to
// the human (or a pending session import) until it is authenticated again.
export function shouldLeaveTabAlone({ url, loggedIn, pageOpen = true }) {
  if (!pageOpen) return false;
  if (isAuthFlowUrl(url)) return true;
  return loggedIn === false && isChatGptUrl(url);
}

// Pick the one tab the worker drives. Prefers an existing ChatGPT tab, then a
// login-flow tab (left alone), then a blank tab to navigate; duplicate ChatGPT
// tabs (left behind by earlier recoveries) are reported so they can be closed.
export function chooseChatPage(pages) {
  const list = Array.isArray(pages) ? pages : [];
  const url = (page) => (typeof page.url === "function" ? page.url() : page.url) || "";
  const chat = list.filter((page) => isChatGptUrl(url(page)));
  const auth = list.find((page) => isAuthFlowUrl(url(page)));
  const blank = list.find((page) => /^(about:blank|chrome:\/\/newtab)/.test(url(page)));
  if (chat.length) return { chosen: chat[0], duplicates: chat.slice(1), navigate: false };
  if (auth) return { chosen: auth, duplicates: [], navigate: false };
  if (blank) return { chosen: blank, duplicates: [], navigate: true };
  return { chosen: null, duplicates: [], navigate: true };
}

async function getChatPage(context) {
  const { chosen, duplicates, navigate } = chooseChatPage(context.pages());
  for (const extra of duplicates) {
    await extra.close().catch(() => {});
  }
  if (duplicates.length) log(`closed ${duplicates.length} duplicate ChatGPT tab(s)`);
  let page = chosen;
  if (!page) page = await context.newPage();
  if (navigate) await page.goto("https://chatgpt.com/", { waitUntil: "domcontentloaded" });
  if (isChatGptUrl(page.url())) await installDomCore(page);
  return page;
}

// Stop the Chrome instance bound to this worker's profile so a relaunch is a
// real relaunch. Playwright's browser.close() on a CDP-attached browser only
// disconnects; spawning again then hands the URL to the still-running
// instance, which is how relaunch loops used to open a new tab every time.
function terminateChrome() {
  if (!CHROME_PROFILE_DIR) return 0;
  const marker = `--user-data-dir=${CHROME_PROFILE_DIR}`;
  let pids = [];
  try {
    pids = execFileSync("pgrep", ["-f", "--", marker], { encoding: "utf8" })
      .split(/\s+/)
      .map((value) => Number(value))
      .filter((value) => Number.isInteger(value) && value > 0 && value !== process.pid);
  } catch {
    return 0;
  }
  for (const pid of pids) {
    try {
      process.kill(pid, "SIGTERM");
    } catch {}
  }
  return pids.length;
}

async function waitForChromeExit(timeoutMs = 5000) {
  const marker = `--user-data-dir=${CHROME_PROFILE_DIR}`;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      execFileSync("pgrep", ["-f", "--", marker], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
    } catch {
      return true; // pgrep exits non-zero when nothing matches
    }
    await sleep(250);
  }
  try {
    for (const pid of execFileSync("pgrep", ["-f", "--", marker], { encoding: "utf8" }).split(/\s+/)) {
      const value = Number(pid);
      if (Number.isInteger(value) && value > 0) process.kill(value, "SIGKILL");
    }
  } catch {}
  return false;
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
  ];
}

// Name the Chrome profile so the window's avatar/profile menu (and
// chrome://version) identify which pool/worker it belongs to.
export function seedProfileName(profileDir, name, fs = { existsSync, mkdirSync, writeFileSync }) {
  const prefs = resolve(profileDir, "Default", "Preferences");
  if (fs.existsSync(prefs)) return false;
  fs.mkdirSync(dirname(prefs), { recursive: true, mode: 0o700 });
  fs.writeFileSync(prefs, JSON.stringify({ profile: { name } }), { mode: 0o600 });
  return true;
}

function launchChrome() {
  if (!CHROME_EXECUTABLE) {
    throw Object.assign(new Error("no Chrome executable configured"), {
      code: "chrome_launch_unconfigured",
    });
  }
  mkdirSync(CHROME_PROFILE_DIR, { recursive: true, mode: 0o700 });
  try {
    seedProfileName(CHROME_PROFILE_DIR, `NyxID Oracle ${LABEL}`);
  } catch {}
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

export function markChatPageRecovered(runtime) {
  runtime.health.tab = 0;
  runtime.chromeAlive = true;
  if (!runtime.state?.current_task) runtime.lastError = null;
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
  watchLoginChanges(runtime);
  runtime.page = await getChatPage(runtime.context);
  runtime.health.cdp = 0;
  markChatPageRecovered(runtime);
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
        const stopped = terminateChrome();
        if (stopped) {
          log(`stopping ${stopped} Chrome process(es) for a real relaunch`);
          await waitForChromeExit();
        }
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
    } else if (!isChatGptUrl(runtime.page.url()) && !isAuthFlowUrl(runtime.page.url())) {
      await runtime.page.goto("https://chatgpt.com/", {
        waitUntil: "domcontentloaded",
        timeout: 60000,
      });
      await installDomCore(runtime.page);
    }
    markChatPageRecovered(runtime);
    return runtime.page;
  } catch (error) {
    runtime.health.tab += 1;
    runtime.lastError = stableErrorCode(error);
    if (runtime.health.tab >= 3) {
      runtime.health.cdp += 1;
      return recoverChrome(runtime, runtime.health.tab >= 5);
    }
    try {
      const stale = runtime.page;
      runtime.page = await runtime.context.newPage();
      if (stale && !stale.isClosed()) await stale.close().catch(() => {});
      await runtime.page.goto(targetUrl || "https://chatgpt.com/", {
        waitUntil: "domcontentloaded",
        timeout: 60000,
      });
      await installDomCore(runtime.page);
      markChatPageRecovered(runtime);
      return runtime.page;
    } catch (replacementError) {
      runtime.health.cdp += 1;
      runtime.lastError = stableErrorCode(replacementError);
      return recoverChrome(runtime);
    }
  }
}

// ── Prompt flow ──────────────────────────────────────────────────────────
// Map a requested model label to the ChatGPT picker's reasoning levels. The
// current UI exposes Instant/Medium/High/Extra High/Pro as role="menuitemradio"
// entries; "-pro" (the pool default `chatgpt-5.5-pro`) selects the Pro level.
// Chinese labels are kept so either localisation matches. The first entry is
// the canonical display name.
export function modelLevelTargets(label) {
  const raw = String(label || "").trim();
  if (!raw) return [];
  const lower = raw.toLowerCase();
  const compact = lower.replace(/^(chatgpt|openai)-/, "").replace(/[\s._-]+/g, "");
  if (/\bpro\b|pro$|扩展|extended/.test(lower) || compact.endsWith("pro")) {
    return ["Pro", "Pro 扩展", "扩展"];
  }
  if (/extra\s*high|ultra|超高/.test(lower)) return ["Extra High", "超高"];
  if (/\bhigh\b|高级|advanced/.test(lower)) return ["High", "高级"];
  if (/medium|balanced|均衡/.test(lower)) return ["Medium", "均衡"];
  if (/instant|fast|极速/.test(lower)) return ["Instant", "极速"];
  return [raw];
}

function normalizeMenuText(value) {
  return String(value || "").toLowerCase().replace(/[\s._-]+/g, "");
}

// Exact pass first so "High" never selects "Extra High"; fuzzy pass second.
export function modelItemMatches(itemText, targets, exact) {
  const candidate = normalizeMenuText(itemText);
  if (!candidate) return false;
  const wanted = (targets || []).map(normalizeMenuText).filter(Boolean);
  if (!exact && MODEL_LEVELS.some((aliases) => aliases[0] === targets?.[0]) &&
      detectPillLevel(itemText) !== targets[0]) return false;
  return exact
    ? wanted.some((w) => candidate === w)
    : wanted.some((w) => candidate.includes(w) || w.includes(candidate));
}

const MODEL_SELECT_TIMEOUT_MS = Math.max(1, Math.min(25000,
  Number(process.env.NYXID_MODEL_SELECT_TIMEOUT_MS) || 25000));
const PRE_SEND_ACTION_MS = 5000;
const COMPOSER_SELECTOR = "#prompt-textarea, div[contenteditable='true'][role='textbox'], textarea[data-testid='prompt-textarea']";
const SEND_SELECTOR = "button[data-testid='send-button'], button[aria-label='Send prompt'], button[aria-label='发送提示']";
const PILL_SELECTOR = 'button.__composer-pill[aria-haspopup="menu"]:visible';
const COMPOSER_REGION_XPATH = "xpath=ancestor::*[.//button[@data-testid='send-button' or @aria-label='Send prompt' or @aria-label='发送提示']][1]";

const MODEL_LEVELS = [
  ["Extra High", "超高"],
  ["Pro", "Pro 扩展", "扩展"],
  ["High", "高级"],
  ["Medium", "均衡"],
  ["Instant", "极速"],
];

// Canonical levels only, with word boundaries so e.g. "Profile" is not Pro.
// Preserve the existing Chinese aliases; structural discovery handles locale.
export function detectPillLevel(text) {
  const label = String(text || "").toLowerCase().replace(/[._-]+/g, " ");
  for (const aliases of MODEL_LEVELS) {
    const english = aliases[0].toLowerCase().replace(/ /g, "\\s*");
    if (new RegExp(`(?:^|[^a-z])${english}(?:$|[^a-z])`).test(label) ||
        aliases.slice(1).some((alias) => label.includes(alias.toLowerCase()))) return aliases[0];
  }
  return null;
}

export function pillShowsLevel(pillText, targets) {
  const canonical = (targets || [])[0];
  if (!canonical || !pillText) return false;
  if (MODEL_LEVELS.some((aliases) => aliases[0] === canonical)) {
    return detectPillLevel(pillText) === canonical;
  }
  return normalizeMenuText(pillText).includes(normalizeMenuText(canonical));
}

// Index in a snapshot of visible entries. Never commit an arbitrary first
// item, even if checked. A submenu may contain account actions, not levels.
export function chooseNestedLevelEntry(items, targets, allowChecked = true) {
  const recognized = (item) => detectPillLevel(item.text) !== null;
  for (const exact of [true, false]) {
    const index = items.findIndex((item) => recognized(item) && modelItemMatches(item.text, targets, exact));
    if (index >= 0) return index;
  }
  return allowChecked ? items.findIndex((item) => recognized(item) && item.checked) : -1;
}

export function reportedPromptModel(task) {
  // model_selected is exclusively an observed pill label, never a clicked
  // item or a claim of verification. Retain the request when no pill is read.
  return task.model_selected || task.model;
}

export function modelSelectionDetail(result) {
  const level = MODEL_LEVELS.some((aliases) => aliases[0] === result.level) ? result.level : "custom";
  if (result.reason === "timeout") return "timeout";
  if (result.verified) return `selected=${level}`;
  if (result.reason === "unverified") return `unverified=${level}`;
  return result.reason;
}

// Use the same preference for structural pills and composer-local fallbacks.
export function preferredModelPillIndex(labels) {
  if (!labels.length) return -1;
  const recognized = labels.findIndex((text) => detectPillLevel(text) !== null);
  if (recognized >= 0) return recognized;
  const legacy = labels.findIndex((text) => /instant|medium|high|extra|pro|gpt|思考|扩展|极速|均衡|高级|超高|\b5(\.|\b)/i.test(text));
  return legacy >= 0 ? legacy : 0;
}

export function modelSelectionDiagnostics(snapshot) {
  const source = snapshot?.pill ? (snapshot.pill.structural ? "structural" : "fallback") : "none";
  const observed = snapshot?.observed || "";
  const items = snapshot?.items || [];
  const recognized = [...new Set(items.map((item) => detectPillLevel(item.text)).filter(Boolean))];
  return `pill_source=${source} pill_level=${detectPillLevel(observed) || "unrecognized"} ` +
    `pill_text_length=${observed.length} items=${items.length} recognized=[${recognized.join(",")}]`;
}

function interactionDeadlineError() {
  return Object.assign(new Error("interaction_deadline"), { code: "interaction_deadline" });
}

export function requireInteractionRead(value) {
  if (value === null) throw interactionDeadlineError();
  return value;
}

export function modelSelectionFailureReason(error, { deadline, aborted }, now) {
  if (aborted || now >= deadline) return "timeout";
  return error?.code === "interaction_deadline" ? "interaction_deadline" : "selection_failed";
}

function interactionBudget(duration, picker = null) {
  return { deadline: Date.now() + duration, controller: new AbortController(), picker };
}

function interactionOptions(budget, maximum = 3000) {
  const remaining = budget.deadline - Date.now();
  if (remaining <= 0 || budget.controller.signal.aborted) {
    budget.controller.abort();
    throw interactionDeadlineError();
  }
  return { timeout: Math.max(1, Math.min(maximum, remaining)), signal: budget.controller.signal };
}

async function budgetPause(budget, ms) {
  await sleep(interactionOptions(budget, ms).timeout);
  interactionOptions(budget);
}

// Locator.evaluate's timeout covers resolution, not the evaluation itself.
// Bound read-only evaluations too; their browser callback checks the same
// deadline before reading DOM, and no continuation may act after abort.
async function boundedRead(budget, read) {
  const { timeout, signal } = interactionOptions(budget, 1000);
  let timer;
  let abort;
  try {
    return requireInteractionRead(await Promise.race([
      read(timeout),
      new Promise((_, reject) => {
        abort = () => reject(interactionDeadlineError());
        signal.addEventListener("abort", abort, { once: true });
        timer = setTimeout(abort, timeout);
      }),
    ]));
  } finally {
    clearTimeout(timer);
    signal.removeEventListener("abort", abort);
  }
}

// Synchronous, read-only snapshots avoid N per-item auto-waits. Discovery is
// restricted to the structural pill or the textarea's own composer region.
// No arbitrary menu labels are written to logs or acknowledgement metadata.
async function pickerSnapshot(page, budget) {
  const snapshot = await boundedRead(budget, (timeout) => page.locator("body").evaluate((body, { composerSelector, sendSelector, deadline, pickerId }) => {
    if (Date.now() >= deadline) return null;
    const visible = (el) => {
      const rect = el.getBoundingClientRect();
      const style = getComputedStyle(el);
      return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none";
    };
    const input = body.querySelector(composerSelector);
    const form = input?.closest("form");
    let region = form || input?.parentElement;
    if (!form) while (region && !region.querySelector(sendSelector)) region = region.parentElement;
    if (region === body || region === document.documentElement) region = null;
    const pills = [...body.querySelectorAll('button.__composer-pill[aria-haspopup="menu"]')].filter(visible);
    const candidates = pills.length ? pills : [...(region?.querySelectorAll('button[aria-haspopup="menu"]') || [])].filter(visible);
    const menus = window.__nyx.modelPickerMenus(pickerId);
    const items = window.__nyx.modelPickerItems(pickerId).map((el) => ({
      text: (el.innerText || el.textContent || "").trim(),
      checked: el.getAttribute("aria-checked") === "true" || el.getAttribute("aria-selected") === "true",
    }));
    return {
      candidates: candidates.map((el) => (el.innerText || el.textContent || "").trim()),
      structural: !!pills.length, form: !!form,
      open: menus.length > 0, items, submenu: !!window.__nyx.modelPickerTrigger(pickerId),
    };
  }, { composerSelector: COMPOSER_SELECTOR, sendSelector: SEND_SELECTOR, deadline: Date.now() + timeout,
    pickerId: budget.picker?.id }, interactionOptions(budget, 1000)));
  interactionOptions(budget);
  const index = preferredModelPillIndex(snapshot.candidates);
  snapshot.pill = index < 0 ? null : { index, structural: snapshot.structural, form: snapshot.form };
  snapshot.observed = snapshot.candidates[index] || null;
  if (budget.picker) {
    // Preserve the last open picker's items after Escape for diagnostics.
    // These texts stay in memory only; logging projects canonical metadata.
    const lastItems = budget.picker.snapshot?.items || [];
    budget.picker.snapshot = { ...snapshot, items: snapshot.open ? snapshot.items : lastItems };
  }
  return snapshot;
}

async function beginModelPicker(page, budget) {
  await boundedRead(budget, (timeout) => page.locator("body").evaluate((_, { id, deadline }) => {
    if (Date.now() >= deadline) return null;
    return window.__nyx.beginModelPicker(id);
  }, { id: budget.picker.id, deadline: Date.now() + timeout }, interactionOptions(budget, 1000)));
}

function pickerLocator(page, pill) {
  if (pill.structural) return page.locator(PILL_SELECTOR).nth(pill.index);
  const region = page.locator(COMPOSER_SELECTOR).first().locator(pill.form ? "xpath=ancestor::form[1]" : COMPOSER_REGION_XPATH);
  return region.locator('button[aria-haspopup="menu"]:visible').nth(pill.index);
}

async function clickPickerElement(page, budget, entry) {
  let handle;
  let acceptingHandle = true;
  try {
    handle = await boundedRead(budget, (timeout) => page.locator("body").evaluateHandle((_, { id, deadline, entry }) => {
      if (Date.now() >= deadline) return null;
      return entry.trigger ? window.__nyx.modelPickerTrigger(id)
        : window.__nyx.modelPickerItem(id, entry.index, entry.text);
    }, { id: budget.picker.id, deadline: Date.now() + timeout, entry }, interactionOptions(budget, 1000)).then((value) => {
      // If evaluation completed after the read/selection deadline, release its
      // handle without allowing a late click or leaking a remote reference.
      if (!acceptingHandle || budget.controller.signal.aborted) {
        void value.dispose().catch(() => {});
        throw interactionDeadlineError();
      }
      return value;
    }));
    interactionOptions(budget);
    const element = handle.asElement();
    if (!element) throw Object.assign(new Error("picker_changed"), { code: "picker_changed" });
    await element.click(interactionOptions(budget));
  } finally {
    acceptingHandle = false;
    await handle?.dispose().catch(() => {});
  }
}

async function clickMatchingLevel(page, targets, budget, allowChecked = false) {
  const snapshot = await pickerSnapshot(page, budget);
  const index = chooseNestedLevelEntry(snapshot.items, targets, allowChecked);
  if (index < 0) return false;
  // Revalidate innerText and membership page-side, then click that exact node.
  // Hidden hints in textContent cannot invalidate a visible level match.
  await clickPickerElement(page, budget, { index, text: snapshot.items[index].text });
  return true;
}

async function closeOpenMenus(page, budget) {
  for (let escapes = 0; escapes < 3 && (await pickerSnapshot(page, budget)).open; escapes += 1) {
    await page.locator("body").press("Escape", interactionOptions(budget));
    await budgetPause(budget, 100);
  }
}

// Returns { level, verified, observed, reason }. Only the actual pill can
// verify a level or populate observed. Selection never throws into the task
// flow. Abort cancels Playwright actions, and the deadline is checked before
// EVERY interaction, including after reads that resolve late. The backstop
// drains the inner promise before menu cleanup; no detached selection loop
// can race prompt typing or Send.
async function selectModel(page, modelLabel) {
  const targets = modelLevelTargets(modelLabel);
  const result = { level: targets[0] || null, verified: false, observed: null, reason: "picker_unavailable" };
  const budget = interactionBudget(MODEL_SELECT_TIMEOUT_MS, { id: randomUUID(), snapshot: null });
  let timer;
  let drainTimer;
  const inner = selectModelInner(page, targets, budget, result).catch((error) => {
    result.reason = modelSelectionFailureReason(error, {
      deadline: budget.deadline, aborted: budget.controller.signal.aborted,
    }, Date.now());
  });
  const timeout = new Promise((resolveTimeout) => {
    timer = setTimeout(() => {
      budget.controller.abort(); // cancel even a click waiting for actionability
      resolveTimeout("timeout");
    }, MODEL_SELECT_TIMEOUT_MS);
  });
  try {
    if (await Promise.race([inner, timeout]) === "timeout") {
      await Promise.race([inner, new Promise((resolveDrain) => { drainTimer = setTimeout(resolveDrain, 3000); })]);
      result.reason = "timeout";
    }
  } finally {
    budget.controller.abort();
    clearTimeout(timer);
    clearTimeout(drainTimer);
  }
  // Cleanup gets its own small budget after the aborted selection is drained.
  // The pre-send guard below also handles non-menu overlays and stuck Radix
  // body pointer-events. Cleanup failure must not consume a recovery attempt.
  const cleanup = interactionBudget(2000, budget.picker);
  const cleanupTimer = setTimeout(() => cleanup.controller.abort(), 2000);
  try {
    await closeOpenMenus(page, cleanup);
    result.observed = (await pickerSnapshot(page, cleanup)).observed;
  } catch {} finally {
    cleanup.controller.abort();
    clearTimeout(cleanupTimer);
  }
  result.verified = pillShowsLevel(result.observed, targets);
  if (result.verified && !["timeout", "already_selected"].includes(result.reason)) result.reason = "selected";
  log(`model_selection reason=${result.reason} ${modelSelectionDetail(result)} ${modelSelectionDiagnostics(budget.picker.snapshot)}`);
  return { ...result };
}

async function selectModelInner(page, targets, budget, result) {
  const before = await pickerSnapshot(page, budget);
  interactionOptions(budget);
  result.observed = before.observed;
  if (pillShowsLevel(before.observed, targets)) {
    result.reason = "already_selected";
    return;
  }
  if (!before.pill || !targets.length) return;
  await beginModelPicker(page, budget);
  await pickerLocator(page, before.pill).click(interactionOptions(budget));
  try {
    await page.locator("body").waitForFunction((_, id) => window.__nyx.modelPickerMenus(id).length > 0,
      budget.picker.id, interactionOptions(budget, 5000));
  } catch (error) {
    interactionOptions(budget);
    if (error?.name !== "TimeoutError") throw error;
    result.reason = "menu_not_opened";
    return;
  }
  let clicked = await clickMatchingLevel(page, targets, budget);
  if (!clicked && (await pickerSnapshot(page, budget)).submenu) {
    await clickPickerElement(page, budget, { trigger: true });
    await budgetPause(budget, 200);
    clicked = await clickMatchingLevel(page, targets, budget);
  }
  if (!clicked) {
    interactionOptions(budget);
    result.reason = "level_unavailable";
    return;
  }
  await budgetPause(budget, 200);
  if ((await pickerSnapshot(page, budget)).open) await clickMatchingLevel(page, targets, budget, true);
  await closeOpenMenus(page, budget);
  // Allow up to one second for the pill to reflect the click, without
  // repeatedly reopening the picker or treating clicked text as observation.
  const verifyUntil = Math.min(budget.deadline, Date.now() + 1000);
  while (true) {
    const after = await pickerSnapshot(page, budget);
    interactionOptions(budget);
    result.observed = after.observed;
    if (pillShowsLevel(result.observed, targets) || Date.now() >= verifyUntil) break;
    await budgetPause(budget, 100);
  }
  result.reason = "unverified";
}

// Clear overlays before typing and again immediately before Send. Never force
// a click through an obstruction: failure stays pre-send and enters the
// existing browser recovery / infrastructure retry path.
async function ensureComposerUnobstructed(page) {
  const budget = interactionBudget(PRE_SEND_ACTION_MS);
  const timer = setTimeout(() => budget.controller.abort(), PRE_SEND_ACTION_MS);
  try {
    await page.locator(COMPOSER_SELECTOR).first().scrollIntoViewIfNeeded(interactionOptions(budget)).catch(() => {});
    while (true) {
      const state = await boundedRead(budget, (timeout) => page.locator("body").evaluate((body, { composerSelector, deadline }) => {
        if (Date.now() >= deadline) return null;
        const input = body.querySelector(composerSelector);
        const rect = input?.getBoundingClientRect();
        const hit = rect && document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
        const open = [...body.querySelectorAll('[role="menu"], [role="listbox"]')].some((el) => {
          const r = el.getBoundingClientRect();
          const s = getComputedStyle(el);
          return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none";
        });
        const main = body.querySelector("main");
        const mainRect = main?.getBoundingClientRect();
        const neutral = mainRect && document.elementFromPoint(mainRect.x + 4, mainRect.y + 4) === main;
        return { clear: !!hit && input.contains(hit) && getComputedStyle(body).pointerEvents !== "none", open, neutral };
      }, { composerSelector: COMPOSER_SELECTOR, deadline: Date.now() + timeout }, interactionOptions(budget, 1000)));
      interactionOptions(budget);
      if (state.clear) return;
      await page.locator("body").press("Escape", interactionOptions(budget));
      if (!state.open && state.neutral) {
        await page.locator("main").first().click({ position: { x: 4, y: 4 }, ...interactionOptions(budget) }).catch(() => {});
      }
      await budgetPause(budget, 100);
    }
  } catch {
    log("composer_unobstructed_failed");
    throw Object.assign(new Error("composer_unobstructed_failed"), { code: "composer_unobstructed_failed" });
  } finally {
    budget.controller.abort();
    clearTimeout(timer);
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
    const attach = page.locator("button[aria-label='Attach files'], button[aria-label='Upload file'], button[data-testid='composer-attach-button']").first();
    if (await attach.count()) { await attach.click({ timeout: PRE_SEND_ACTION_MS }).catch(() => {}); await sleep(800); }
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
    const attach = page.locator("button[aria-label='Attach files'], button[aria-label='Upload file'], button[data-testid='composer-attach-button']").first();
    if (await attach.count()) { await attach.click({ timeout: PRE_SEND_ACTION_MS }).catch(() => {}); await sleep(800); }
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
      files: window.__nyx?.extractFiles() || [],
    };
  });
}

async function submitPromptResult(
  runtime,
  page,
  task,
  text,
  imageSources = [],
  fileSources = []
) {
  updateTaskState(runtime.state, { phase: "settling", conversation_url: page.url() });
  const downloadedImages = await downloadImages(
    page,
    imageSources,
    MAX_ARTIFACTS_TOTAL_BYTES
  );
  const downloadedFiles = await downloadFiles(
    page,
    fileSources,
    MAX_ARTIFACTS_TOTAL_BYTES - downloadedImages.bytes
  );
  const response = text || "";
  if (
    !response.trim() &&
    downloadedImages.items.length === 0 &&
    downloadedFiles.items.length === 0
  ) {
    throw new TaskFailure("empty_extraction");
  }
  const result = await apiPost(
    "/result",
    taskIdentity(runtime, task, {
      response,
      images: downloadedImages.items,
      files: downloadedFiles.items,
      chatgpt_url: page.url(),
      // The observed pill is useful even when selection was unverified.
      model: reportedPromptModel(task),
    })
  );
  log(
    `prompt ${task.task_id} -> ${result.status} (${response.length} chars, ` +
      `${downloadedImages.items.length} image(s)/${downloadedImages.bytes}B, ` +
      `${downloadedFiles.items.length} file(s)/${downloadedFiles.bytes}B)`
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
  if (await ack(runtime, task, "page_ready")) throw new TaskFailure("cancelled");
  if (await recoverPreSendLogin(runtime)) throw new TaskRestart();

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
      await submitPromptResult(
        runtime,
        page,
        task,
        decision.response,
        snapshot.images,
        snapshot.files
      );
      return;
    }
    if (decision.action === "wait") {
      const beforeCount = snapshot.turns
        .slice(0, runtime.state.current_task?.baseline_turn_count || 0)
        .filter((turn) => turn.role === "assistant").length;
      const answer = await waitForResponse(runtime, page, task, beforeCount);
      await submitPromptResult(
        runtime,
        page,
        task,
        answer.text,
        answer.images,
        answer.files
      );
      return;
    }
    if (decision.action === "uncertain") {
      throw new TaskFailure("prompt_delivery_uncertain");
    }
  }

  if (task.model && task.model !== "unknown") {
    if (await ack(runtime, task, "selecting_model")) throw new TaskFailure("cancelled");
    const selected = await selectModel(page, task.model);
    task.model_selected = selected.observed;
    if (await ack(runtime, task, "selecting_model", modelSelectionDetail(selected))) {
      throw new TaskFailure("cancelled");
    }
  }

  // Type the prompt into the composer (native — more robust than the
  // userscript's execCommand fallbacks) and send.
  const input = page.locator(COMPOSER_SELECTOR).first();
  await input.waitFor({ state: "visible", timeout: 60000 });
  await ensureComposerUnobstructed(page);
  await input.click({ timeout: PRE_SEND_ACTION_MS });
  await input.fill(task.prompt, { timeout: PRE_SEND_ACTION_MS });
  const before = await boundedRead(interactionBudget(PRE_SEND_ACTION_MS), (timeout) =>
    page.locator("body").evaluate(() => ({
      baseline: window.__nyx?.extractTranscript()?.length || 0,
      assistantCount: window.__nyx.assistantCount(),
    }), undefined, { timeout }));
  const baseline = before.baseline;
  updateTaskState(runtime.state, { phase: "ready_to_send", baseline_turn_count: baseline });
  if (await ack(runtime, task, "ready_to_send")) throw new TaskFailure("cancelled");
  await sleep(300);
  // Only attach a PDF on the FIRST turn of a conversation — never re-upload it
  // into an existing chat if the server ever resends pdf_base64 on a follow-up
  // (mirrors the userscript's `!is_followup && pdf_base64` guard).
  if (!task.is_followup && task.pdf_base64) {
    if (await ack(runtime, task, "uploading_pdf")) throw new TaskFailure("cancelled");
    await uploadPdf(runtime, page, task);
  }
  // Same first-turn-only guard for a general attachment (image / pdf / ...).
  if (!task.is_followup && task.attachment_base64) {
    if (await ack(runtime, task, "uploading_attachment")) throw new TaskFailure("cancelled");
    await uploadAttachment(runtime, page, task);
  }

  const beforeCount = before.assistantCount;
  const sendBtn = page.locator(SEND_SELECTOR).first();
  await ensureComposerUnobstructed(page);
  // Resolve actionability while still pre-send. The actual click is the
  // only operation after the durable uncertainty fence.
  await sendBtn.click({ trial: true, timeout: PRE_SEND_ACTION_MS });
  if (!task.is_followup && (task.pdf_base64 || task.attachment_base64)) {
    if (await ack(runtime, task, "ready_to_send")) throw new TaskFailure("cancelled");
  }
  updateTaskState(runtime.state, { phase: "send_attempted", baseline_turn_count: baseline });
  await sendBtn.click({ timeout: PRE_SEND_ACTION_MS });
  updateTaskState(runtime.state, { phase: "sent" });
  await ack(runtime, task, "sent");
  await pinCurrentConversation(runtime, page, task);

  const { text, images, files } = await waitForResponse(runtime, page, task, beforeCount);
  await submitPromptResult(runtime, page, task, text, images, files);
}

function convId(url) {
  const m = (url || "").match(/\/c\/([a-f0-9-]{6,})/);
  return m ? m[1] : null;
}

// Returns the latest assistant turn's text plus on-page image and file sources.
// Artifact-only turns are valid; the stability key spans all three outputs.
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
        runtime.loggedIn === false && canImportLogin(runtime.state, runtime.loggedIn)
      ) {
        await processPendingCommand(runtime, true);
        throw new TaskRestart();
      }
    }
    const [generating, count, text, images, files] = await page.evaluate(() => [
      window.__nyx.isStillGenerating(),
      window.__nyx.assistantCount(),
      window.__nyx.extractResponse(),
      window.__nyx.extractImages(),
      window.__nyx.extractFiles(),
    ]);
    const hasText = !!(text && text.length > 0);
    const hasImages = Array.isArray(images) && images.length > 0;
    const hasFiles = Array.isArray(files) && files.length > 0;
    // A text answer bumps the assistant-role count; an image-generation turn
    // does NOT (its <img> lives in a conversation-turn with no assistant role),
    // so artifacts carry that case through. Until text or an artifact appears
    // there's no new answer yet — wedge guard bails if ChatGPT has stopped.
    if (count <= beforeCount && !hasImages && !hasFiles) {
      if (!generating && Date.now() - start >= NO_OUTPUT_IDLE_MS) {
        throw new Error("no assistant output (idle timeout)");
      }
      continue;
    }
    if (generating) {
      stable = 0;
      continue;
    }
    if (!hasText && !hasImages && !hasFiles) {
      // New turn settled but produced nothing extractable (e.g. an unrenderable
      // tool turn). Don't wedge — fail fast once the idle window elapses.
      if (Date.now() - start >= NO_OUTPUT_IDLE_MS) {
        throw new Error("assistant turn produced no extractable content (idle timeout)");
      }
      stable = 0;
      continue;
    }
    const key =
      (text || "").slice(0, 200) +
      "|" +
      (text || "").length +
      "|" +
      (images || []).join(",") +
      "|" +
      JSON.stringify(files || []);
    if (key === lastKey) {
      stable += 1;
      if (stable >= 2) {
        return { text: text || "", images: images || [], files: files || [] };
      }
    } else {
      stable = 0;
      lastKey = key;
    }
  }
  // Timed out. Only return content if a NEW assistant turn actually appeared
  // since we sent the prompt; otherwise the latest message is stale (a previous
  // turn), so return empty and let the server mark the task failed instead of
  // handing back the wrong answer.
  const [count, text, images, files] = await page.evaluate(() => [
    window.__nyx.assistantCount(),
    window.__nyx.extractResponse(),
    window.__nyx.extractImages(),
    window.__nyx.extractFiles(),
  ]);
  return count > beforeCount || images?.length || files?.length
    ? { text: text || "", images: images || [], files: files || [] }
    : { text: "", images: [], files: [] };
}

async function fetchTrustedArtifact(page, source, kind) {
  let current = source;
  for (let redirect = 0; redirect <= 5; redirect += 1) {
    if (!isTrustedArtifactUrl(current) || current.startsWith("blob:")) return null;
    const response = await page.request.get(current, {
      timeout: 30000,
      maxRedirects: 0,
    });
    const status = response.status();
    if (status >= 300 && status < 400) {
      const location = response.headers().location;
      if (!location) {
        log(`${kind} fetch returned redirect without location`);
        return null;
      }
      try {
        current = new URL(location, current).href;
      } catch {
        return null;
      }
      continue;
    }
    if (!response.ok()) {
      log(`${kind} fetch returned HTTP ${status}`);
      return null;
    }
    return response;
  }
  log(`${kind} fetch exceeded redirect limit`);
  return null;
}

async function downloadBlob(page, source, fallbackMime) {
  const data = await page.evaluate(
    async ({ url, defaultMime }) => {
      const response = await fetch(url);
      const blob = await response.blob();
      const buffer = new Uint8Array(await blob.arrayBuffer());
      let binary = "";
      for (let i = 0; i < buffer.length; i += 1) {
        binary += String.fromCharCode(buffer[i]);
      }
      return { b64: btoa(binary), mime: blob.type || defaultMime };
    },
    { url: source, defaultMime: fallbackMime }
  );
  return { buffer: Buffer.from(data.b64, "base64"), mime: data.mime };
}

// Download the latest turn's images through the browser's authenticated
// context — page.request.get carries the session cookies and isn't subject to
// the same-origin policy, so it can read cross-origin oaiusercontent bytes a
// page fetch() would get only as an opaque (unreadable) response. blob: URLs
// resolve only inside the page, so those are fetched there. Returns the
// worker-API image array; caps mirror the server (which re-validates).
async function downloadImages(page, srcs, artifactBudget) {
  const out = [];
  let total = 0;
  for (const src of (srcs || []).slice(0, MAX_IMAGES)) {
    // Trust boundary (SSRF): this is where the privileged, cookie-bearing fetch
    // happens, so enforce the content-host allowlist HERE — independent of
    // extractImages' heuristics (its alt-based match is model-controlled and
    // could otherwise smuggle an internal URL through). blob: is page-local.
    if (!isTrustedArtifactUrl(src)) continue;
    try {
      let buffer, mime;
      if (src.startsWith("blob:")) {
        ({ buffer, mime } = await downloadBlob(page, src, "image/png"));
      } else {
        const resp = await fetchTrustedArtifact(page, src, "image");
        if (!resp) continue;
        buffer = await resp.body();
        mime = (resp.headers()["content-type"] || "image/png").split(";")[0].trim();
      }
      if (!buffer?.length) continue;
      const decision = artifactBudgetDecision(
        total,
        buffer?.length,
        MAX_IMAGE_BYTES,
        Math.min(MAX_IMAGES_TOTAL_BYTES, artifactBudget)
      );
      if (decision === "skip") {
        log(`image too large (${buffer.length}B), skipping`);
        continue;
      }
      if (decision === "stop") {
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
  return { items: out, bytes: total };
}

// Download generic files with the same cookie-bearing fetch boundary and the
// same shared artifact budget as generated images. Names never enter logs.
async function downloadFiles(page, sources, artifactBudget) {
  const out = [];
  let total = 0;
  for (const source of (sources || []).slice(0, MAX_FILES)) {
    const href = source?.href || "";
    if (!isTrustedArtifactUrl(href)) continue;
    try {
      let buffer, mime;
      if (href.startsWith("blob:")) {
        ({ buffer, mime } = await downloadBlob(
          page,
          href,
          "application/octet-stream"
        ));
      } else {
        const response = await fetchTrustedArtifact(page, href, "file");
        if (!response) continue;
        buffer = await response.body();
        mime = (response.headers()["content-type"] || "application/octet-stream")
          .split(";")[0]
          .trim();
      }
      if (!buffer?.length) continue;
      const decision = artifactBudgetDecision(
        total,
        buffer?.length,
        MAX_FILE_BYTES,
        Math.min(MAX_FILES_TOTAL_BYTES, artifactBudget)
      );
      if (decision === "skip") {
        log(`file too large (${buffer.length}B), skipping`);
        continue;
      }
      if (decision === "stop") {
        log("file total cap reached, skipping remaining");
        break;
      }
      total += buffer.length;
      if (!mime || mime.length > 256 || /[\x00-\x1f\x7f]/.test(mime)) {
        mime = "application/octet-stream";
      }
      out.push({
        name: sanitizeArtifactName(source?.name, out.length + 1),
        mime,
        data_base64: buffer.toString("base64"),
      });
    } catch (e) {
      log(`file download failed (${stableErrorCode(e)})`);
    }
  }
  return { items: out, bytes: total };
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

async function ack(runtime, task, phase, phaseDetail) {
  const response = await apiPost("/ack", taskIdentity(runtime, task, {
    phase,
    phase_detail: phaseDetail,
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
    runtime.state.drain_requested = command.command === "drain";
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

// A manager withdrew a command this worker is still holding (e.g. an upgrade
// to a version that turned out to be broken): drop it before it runs and stop
// draining unless an explicit drain is in force.
export function dropCancelledCommand(runtime, cancelledIds) {
  const ids = Array.isArray(cancelledIds) ? cancelledIds : [];
  const pending = runtime.state?.pending_command;
  if (!pending || !ids.includes(pending.id)) return false;
  runtime.state.pending_command = null;
  runtime.state.draining = Boolean(runtime.state.drain_requested);
  log(`command ${pending.command} was cancelled by a manager before it ran`);
  return true;
}

async function heartbeat(runtime) {
  let loggedIn = null;
  if (runtime.chromeAlive && runtime.page && !runtime.page.isClosed()) {
    loggedIn = await detectLoggedIn(runtime.page);
  }
  runtime.loggedIn = loggedIn;
  await observeSavedLoginTrust(runtime, loggedIn);
  const reports = [...(runtime.state.pending_reports || [])].slice(0, 16);
  const response = await apiPost("/heartbeat", {
    worker: LABEL,
    instance_id: runtime.state.instance_id,
    platform: `${process.platform}-${process.arch}`,
    capabilities: CAPABILITIES,
    logged_in: loggedIn,
    current_task_id: runtime.state.current_task?.task_id || null,
    chrome_alive: runtime.chromeAlive,
    last_error: runtime.lastError || runtime.state.saved_login_error || null,
    command_reports: reports,
  });
  if (reports.length) {
    const sent = new Set(reports.map((report) => report.command_id));
    runtime.state.pending_reports = (runtime.state.pending_reports || []).filter(
      (report) => !sent.has(report.command_id)
    );
    saveState(runtime.state);
  }
  dropCancelledCommand(runtime, response.cancelled_command_ids);
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
  const key = Buffer.from(hkdfSync("sha256", Buffer.from(token), salt, SESSION_INFO, 32));
  let plaintext;
  try {
    const body = ciphertext.subarray(0, ciphertext.length - 16);
    const tag = ciphertext.subarray(ciphertext.length - 16);
    const decipher = createDecipheriv("aes-256-gcm", key, nonce);
    decipher.setAAD(SESSION_AAD);
    decipher.setAuthTag(tag);
    plaintext = Buffer.concat([decipher.update(body), decipher.final()]);
    if (plaintext.length > MAX_SESSION_PLAINTEXT_BYTES) {
      throw new TaskFailure("session_plaintext_too_large");
    }
    return JSON.parse(plaintext.toString("utf8"));
  } catch (error) {
    if (error instanceof TaskFailure) throw error;
    throw new TaskFailure("session_decrypt_failed");
  } finally {
    key.fill(0);
    plaintext?.fill(0);
  }
}

export function encryptSessionEnvelope(snapshot, token) {
  const plaintext = Buffer.from(JSON.stringify(snapshot));
  if (!plaintext.length || plaintext.length > MAX_SESSION_PLAINTEXT_BYTES) {
    plaintext.fill(0);
    throw new TaskFailure("session_plaintext_too_large");
  }
  const salt = randomBytes(32);
  const nonce = randomBytes(12);
  const key = Buffer.from(hkdfSync("sha256", Buffer.from(token), salt, SESSION_INFO, 32));
  try {
    const cipher = createCipheriv("aes-256-gcm", key, nonce);
    cipher.setAAD(SESSION_AAD);
    const ciphertext = Buffer.concat([cipher.update(plaintext), cipher.final(), cipher.getAuthTag()]);
    const envelope = Buffer.from(JSON.stringify({ version: SESSION_FORMAT_VERSION,
      salt_base64: salt.toString("base64"), nonce_base64: nonce.toString("base64"),
      ciphertext_base64: ciphertext.toString("base64") }));
    if (envelope.length > MAX_SESSION_SNAPSHOT_BYTES) throw new TaskFailure("session_envelope_size_invalid");
    return envelope;
  } finally {
    key.fill(0);
    plaintext.fill(0);
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
  if (runtime.state.saved_login) {
    runtime.state.saved_login.status = "external_login";
    saveState(runtime.state);
  }
  return importSessionPayload(runtime, payload);
}

export function validateSessionSnapshot(snapshot) {
  if (snapshot?.version !== SESSION_FORMAT_VERSION || !Array.isArray(snapshot.cookies) ||
      snapshot.cookies.length > 512 || Buffer.byteLength(JSON.stringify(snapshot)) > MAX_SESSION_PLAINTEXT_BYTES) {
    throw new TaskFailure("session_snapshot_invalid");
  }
  const cookies = snapshot.cookies.filter(allowedSessionCookie).map((cookie) => {
    if (typeof cookie.path !== "string" || !cookie.path.startsWith("/") || cookie.path.length > 2048 ||
        typeof cookie.expires !== "number" || !Number.isFinite(cookie.expires) ||
        typeof cookie.httpOnly !== "boolean" || typeof cookie.secure !== "boolean" ||
        !["Strict", "Lax", "None"].includes(cookie.sameSite)) {
      throw new TaskFailure("session_snapshot_invalid");
    }
    return { name: cookie.name, value: cookie.value, domain: cookie.domain, path: cookie.path,
      expires: cookie.expires, httpOnly: cookie.httpOnly, secure: cookie.secure, sameSite: cookie.sameSite,
      ...(typeof cookie.partitionKey === "string" ? { partitionKey: cookie.partitionKey } : {}) };
  });
  if (!cookies.length) throw new TaskFailure("session_snapshot_no_cookies");
  if (snapshot.origins !== undefined && !Array.isArray(snapshot.origins)) throw new TaskFailure("session_snapshot_invalid");
  const origins = (snapshot.origins || []).filter((entry) => allowedSessionOrigin(entry?.origin));
  if (origins.length > 2 || new Set(origins.map((entry) => entry.origin)).size !== origins.length) {
    throw new TaskFailure("session_snapshot_invalid");
  }
  for (const origin of origins) {
    for (const kind of ["local_storage", "session_storage"]) {
      if (origin[kind] !== undefined && (!Array.isArray(origin[kind]) || origin[kind].length > 1024 ||
          origin[kind].some((item) => typeof item?.name !== "string" || typeof item?.value !== "string" ||
            item.name.length > 1024 || item.value.length > MAX_SESSION_PLAINTEXT_BYTES))) {
        throw new TaskFailure("session_snapshot_invalid");
      }
    }
  }
  return { version: SESSION_FORMAT_VERSION, cookies, origins };
}

export function canImportLogin(state, loggedIn) {
  if (!state.current_task) return true;
  return loggedIn === false && ["claimed", "page_ready", "ready_to_send"].includes(state.current_task.phase);
}

async function importSessionPayload(runtime, payload) {
  if (payload.format_version !== SESSION_FORMAT_VERSION) {
    throw new TaskFailure("session_snapshot_version_unsupported");
  }
  const snapshot = validateSessionSnapshot(decryptSessionEnvelope(
    Buffer.from(payload.sealed_blob_base64 || "", "base64"),
    TOKEN
  ));
  return applySessionSnapshot(runtime, snapshot);
}

async function applySessionSnapshot(runtime, snapshot) {
  runtime.applyingLogin = true;
  try {
    return await applySessionSnapshotInner(runtime, snapshot);
  } finally {
    runtime.applyingLogin = false;
  }
}

async function applySessionSnapshotInner(runtime, snapshot) {
  await ensureChatPage(runtime);
  // Stop old account pages before clearing their storage, so running scripts
  // cannot restore the account we are explicitly replacing.
  for (const page of runtime.context.pages()) {
    if (allowedSessionOrigin(new URL(page.url()).origin)) {
      await page.evaluate(() => sessionStorage.clear());
      await page.goto("about:blank");
    }
  }
  const cdp = await runtime.context.newCDPSession(runtime.page);
  try {
    for (const origin of ["https://chatgpt.com", "https://auth.openai.com"]) {
      await cdp.send("Storage.clearDataForOrigin", { origin, storageTypes: "all" });
    }
  } finally { await cdp.detach(); }
  await runtime.context.clearCookies({ domain: /(^|\.)chatgpt\.com$/ });
  await runtime.context.clearCookies({ domain: /(^|\.)openai\.com$/ });
  await runtime.context.addCookies(snapshot.cookies);
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
  let previousPackage = null;
  try {
    previousPackage = readFileSync(packagePath, "utf8");
  } catch {}
  writeFileSync(packageTemp, `${JSON.stringify(packageBody, null, 2)}\n`, { mode: 0o600 });
  chmodSync(packageTemp, 0o600);
  renameSync(packageTemp, packagePath);
  const restorePackage = () => {
    if (previousPackage !== null) {
      try {
        writeFileSync(packagePath, previousPackage, { mode: 0o600 });
      } catch {}
    }
  };
  if (installedDependencyVersion(installDir) === playwrightVersion) {
    log(`upgrade: playwright-core ${playwrightVersion} already installed; skipping npm`);
  } else {
    log(`upgrade: installing playwright-core ${playwrightVersion} via ${NPM_EXECUTABLE}`);
    try {
      await new Promise((resolveInstall, rejectInstall) => {
        const child = spawn(
          NPM_EXECUTABLE,
          ["install", "--omit=dev", "--no-audit", "--no-fund", "--save-exact"],
          {
            cwd: installDir,
            stdio: "inherit",
            env: { ...process.env, PATH: daemonPath({ envPath: process.env.PATH }) },
          }
        );
        const timeout = setTimeout(() => {
          child.kill("SIGTERM");
          rejectInstall(Object.assign(new Error("npm install timed out"), {
            code: "upgrade_dependency_install_timeout",
          }));
        }, NPM_INSTALL_TIMEOUT_MS);
        child.once("error", (error) => {
          clearTimeout(timeout);
          rejectInstall(
            error?.code === "ENOENT"
              ? Object.assign(new Error(`npm not found at ${NPM_EXECUTABLE}`), {
                  code: "upgrade_npm_unavailable",
                })
              : error
          );
        });
        child.once("exit", (code, signal) => {
          clearTimeout(timeout);
          if (code === 0) resolveInstall();
          else rejectInstall(Object.assign(new Error(`npm install failed (${code ?? signal})`), {
            code: "upgrade_dependency_install_failed",
          }));
        });
      });
    } catch (error) {
      restorePackage();
      throw error;
    }
    if (installedDependencyVersion(installDir) !== playwrightVersion) {
      restorePackage();
      throw Object.assign(new Error("npm install did not produce the pinned dependency"), {
        code: "upgrade_dependency_version_mismatch",
      });
    }
  }
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
      !(allowSessionImportDuringLoggedOutTask && command.command === "session_import" &&
        canImportLogin(runtime.state, runtime.loggedIn)))
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
        if (runtime.state.saved_login) {
          runtime.state.saved_login.status = "external_login";
          saveState(runtime.state);
        }
        await ensureChatPage(runtime, "https://chatgpt.com/auth/login");
        resultCode = "login_page_opened";
        break;
      case "session_import":
        if (!CAPABILITIES.includes("session_import_v1")) throw new TaskFailure("command_unsupported");
        resultCode = await importLoginSnapshot(runtime, command);
        break;
      case "upgrade":
        resultCode = await installBundle(command);
        shouldExit = true;
        break;
      default:
        throw new TaskFailure("command_unsupported");
    }
    runtime.state.draining = Boolean(runtime.state.drain_requested) || command.command === "restart" || command.command === "upgrade";
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
    runtime.state.draining = Boolean(runtime.state.drain_requested);
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
      if (await recoverPreSendLogin(runtime)) {
        recovering = true;
        continue;
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
      log(`task ${task.task_id} paused for browser recovery (${runtime.lastError})`);
      if (recovery.action === "fail") {
        runtime.lastError = recovery.code;
        await settleTaskFailure(runtime, task, recovery.code);
        clearTaskState(runtime.state);
        return;
      }
      runtime.health.cdp += 1;
      await recoverChrome(runtime, recovery.forceRelaunch);
      recovering = true;
    }
  }
}

async function recoverPreSendLogin(runtime) {
  if (!runtime.page || runtime.page.isClosed() || !canImportLogin(runtime.state, false)) return false;
  runtime.loggedIn = await detectLoggedIn(runtime.page);
  await observeSavedLoginTrust(runtime, runtime.loggedIn);
  if (runtime.loggedIn !== false) return false;
  await heartbeat(runtime);
  if (runtime.state.pending_command?.command === "session_import") {
    await processPendingCommand(runtime, true);
    runtime.loggedIn = await detectLoggedIn(runtime.page);
    if (runtime.loggedIn) return true;
  }
  return Boolean(await processSavedLogin(runtime, true));
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
    const snapshot = Buffer.from(JSON.stringify(await captureBrowserSession(context, page)));
    mkdirSync(dirname(outputPath), { recursive: true, mode: 0o700 });
    writeFileSync(outputPath, snapshot, { mode: 0o600 });
    chmodSync(outputPath, 0o600);
  } finally {
    await browser.close().catch(() => {});
  }
}

async function captureBrowserSession(context, page) {
  if (!(await detectLoggedIn(page))) throw new TaskFailure("login_capture_logged_out");
  const cookies = (await context.cookies()).filter(allowedSessionCookie);
  const storageState = await context.storageState();
  const origins = storageState.origins.filter((entry) => allowedSessionOrigin(entry.origin))
    .map((entry) => ({ origin: entry.origin, local_storage: entry.localStorage, session_storage: [] }));
  for (const candidate of context.pages()) {
    const storage = await captureStorage(candidate);
    if (!storage) continue;
    const existing = origins.find((item) => item.origin === storage.origin);
    if (existing) existing.session_storage = storage.session_storage;
    else origins.push(storage);
  }
  if (!(await detectLoggedIn(page))) throw new TaskFailure("login_capture_logged_out");
  return validateSessionSnapshot({ version: SESSION_FORMAT_VERSION, cookies, origins });
}

export function accountFingerprint(accountId, token) {
  return createHmac("sha256", token).update("nyxid-oracle-account-v1\0").update(accountId).digest("hex");
}

async function browserAccountFingerprint(runtime) {
  if (!runtime.page || new URL(runtime.page.url()).origin !== "https://chatgpt.com") return null;
  const accountId = await runtime.page.evaluate(async () => {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 5000);
    try {
      const response = await fetch("/api/auth/session", { credentials: "same-origin", cache: "no-store", redirect: "error", signal: controller.signal });
      if (!response.ok || !response.body) return null;
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let size = 0;
      let body = "";
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > 64 * 1024) { await reader.cancel(); return null; }
        body += decoder.decode(value, { stream: true });
      }
      body += decoder.decode();
      const id = JSON.parse(body)?.user?.id;
      return typeof id === "string" && id.length > 0 && id.length <= 256 ? id : null;
    } catch { return null; }
    finally { clearTimeout(timeout); }
  }).catch(() => null);
  return accountId ? accountFingerprint(accountId, TOKEN) : null;
}

export function savedLoginDecision(state, desired, loggedIn, now = Date.now()) {
  if (desired.status !== "available") return "unavailable";
  if (!canImportLogin(state, loggedIn)) return "defer";
  const local = state.saved_login;
  const sameBinding = local?.profile_id === desired.profile.id && local?.binding_id === desired.binding.binding_id;
  const sameGeneration = sameBinding && local.generation === desired.profile.generation;
  if (!sameBinding && loggedIn === true && !desired.binding.replace_existing) return "preserve_existing";
  if (sameGeneration && local.status === "external_login") return "preserve_existing";
  if (sameGeneration && local.status === "untrusted") return loggedIn === false ? "import" : "preserve_existing";
  if (sameGeneration && local.attempted_revision === desired.profile.revision && local.status !== "verified") return "failed_revision";
  if (sameGeneration && ["failed", "importing"].includes(local.status) && local.attempted_revision !== desired.profile.revision) return "import";
  if (!sameGeneration || (loggedIn === false && local.attempted_revision !== desired.profile.revision)) return "import";
  if (local.status !== "verified" || loggedIn !== true) return "defer";
  if (local.source_revision !== desired.profile.revision) {
    const updated = Date.parse(desired.profile.updated_at);
    if (!state.current_task && Number.isFinite(updated) && now - updated >= 3 * SAVED_LOGIN_REFRESH_MS) return "import";
    return "sibling_revision";
  }
  return state.current_task ? "defer" : "refresh";
}

async function processSavedLogin(runtime, force = false) {
  if (!CAPABILITIES.includes("saved_login_v1")) return;
  if (!force && Date.now() - (runtime.lastSavedLoginPollAt || 0) < SAVED_LOGIN_POLL_MS) return;
  runtime.lastSavedLoginPollAt = Date.now();
  const identity = { worker: LABEL, instance_id: runtime.state.instance_id };
  const local = runtime.state.saved_login;
  const query = new URLSearchParams(identity);
  const observedRevision = runtime.savedLoginObservedRevision || local?.source_revision || local?.attempted_revision;
  if (observedRevision) query.set("known_revision", observedRevision);
  try {
    // Saved-login maintenance uses one bounded HTTP attempt. A failed refresh
    // must not prevent an otherwise healthy browser from serving tasks.
    let desired = await apiRequest("GET", `/login-profile?${query}`, undefined, false);
    if (desired.status === "unbound") {
      if (local) { runtime.state.saved_login = null; saveState(runtime.state); }
      setSavedLoginError(runtime, null);
      return;
    }
    if (desired.status !== "available") {
      setSavedLoginError(runtime, `saved_login_${desired.status === "expired" ? "expired" : "token_changed"}`);
      return;
    }
    runtime.savedLoginObservedRevision = desired.profile.revision;
    runtime.loggedIn = await detectLoggedIn(runtime.page);
    await observeSavedLoginTrust(runtime, runtime.loggedIn);
    if (local?.pending_publication_id && desired.profile.revision === local.pending_publication_id &&
        local.generation === desired.profile.generation && local.binding_id === desired.binding.binding_id) {
      local.source_revision = local.pending_publication_id;
      local.pending_publication_id = null;
      setSavedLoginError(runtime, null);
      saveState(runtime.state);
    }
    const decision = savedLoginDecision(runtime.state, desired, runtime.loggedIn);
    if (decision === "preserve_existing") {
      if (runtime.state.saved_login_error !== "saved_login_account_changed") setSavedLoginError(runtime, "saved_login_existing_account_preserved");
      return;
    }
    if (decision === "failed_revision") return;
    if (decision === "import") {
      setSavedLoginError(runtime, null);
      if (!desired.sealed_blob_base64) desired = await apiRequest("GET", `/login-profile?${new URLSearchParams(identity)}`, undefined, false);
      runtime.loggedIn = await detectLoggedIn(runtime.page);
      if (savedLoginDecision(runtime.state, desired, runtime.loggedIn) !== "import") return;
      // Validate before journaling or changing any live browser state.
      try {
        validateSessionSnapshot(decryptSessionEnvelope(Buffer.from(desired.sealed_blob_base64 || "", "base64"), TOKEN));
      } catch (error) {
        runtime.state.saved_login = { profile_id: desired.profile.id, binding_id: desired.binding.binding_id,
          generation: desired.profile.generation, source_revision: null,
          account_fingerprint: local?.generation === desired.profile.generation ? local?.account_fingerprint : null,
          attempted_revision: desired.profile.revision, status: "failed" };
        saveState(runtime.state);
        throw error;
      }
      const confirmed = await apiRequest("GET", `/login-profile?${new URLSearchParams({ ...identity, known_revision: desired.profile.revision })}`, undefined, false);
      if (confirmed.status !== "available" || confirmed.profile?.revision !== desired.profile.revision ||
          confirmed.profile?.generation !== desired.profile.generation || confirmed.binding?.binding_id !== desired.binding.binding_id) return;
      const handover = local?.profile_id === desired.profile.id && local?.binding_id === desired.binding.binding_id &&
        local?.generation === desired.profile.generation && runtime.loggedIn === true;
      if (handover) {
        const currentIdentity = await browserAccountFingerprint(runtime);
        if (!currentIdentity || !local.account_fingerprint) { setSavedLoginError(runtime, "saved_login_identity_unavailable"); return; }
        if (currentIdentity !== local.account_fingerprint) {
          local.status = "external_login";
          setSavedLoginError(runtime, "saved_login_account_changed");
          saveState(runtime.state);
          return;
        }
      }
      const backup = handover ? await captureBrowserSession(runtime.context, runtime.page) : null;
      if (handover && await browserAccountFingerprint(runtime) !== local.account_fingerprint) {
        setSavedLoginError(runtime, "saved_login_identity_unavailable");
        return;
      }
      runtime.state.saved_login = { profile_id: desired.profile.id, binding_id: desired.binding.binding_id,
        generation: desired.profile.generation, source_revision: null,
        account_fingerprint: handover ? local.account_fingerprint : null,
        attempted_revision: desired.profile.revision, status: "importing", last_export_at: Date.now() };
      saveState(runtime.state);
      try {
        await importSessionPayload(runtime, desired);
        const fingerprint = await browserAccountFingerprint(runtime);
        if (handover && fingerprint !== local.account_fingerprint) {
          throw new TaskFailure(fingerprint ? "saved_login_account_mismatch" : "saved_login_identity_unavailable");
        }
        runtime.state.saved_login.source_revision = desired.profile.revision;
        runtime.state.saved_login.status = "verified";
        runtime.state.saved_login.account_fingerprint = fingerprint;
        runtime.loggedIn = true;
        runtime.lastError = null;
        setSavedLoginError(runtime, fingerprint ? null : "saved_login_identity_unavailable");
      } catch (error) {
        runtime.state.saved_login.status = "failed";
        let importError = stableErrorCode(error);
        if (backup) {
          try {
            await applySessionSnapshot(runtime, backup);
            runtime.loggedIn = true;
          } catch {
            runtime.loggedIn = false;
            importError = "saved_login_restore_failed";
          }
        }
        setSavedLoginError(runtime, importError);
      }
      saveState(runtime.state);
      return runtime.state.saved_login.status === "verified";
    }
    if (decision !== "refresh" || Date.now() - (local.last_export_at || 0) < SAVED_LOGIN_REFRESH_MS) return;
    local.last_export_at = Date.now();
    saveState(runtime.state);
    const beforeIdentity = await browserAccountFingerprint(runtime);
    if (!local.account_fingerprint || !beforeIdentity) {
      setSavedLoginError(runtime, "saved_login_identity_unavailable");
      return;
    }
    if (beforeIdentity !== local.account_fingerprint) {
      local.status = "external_login";
      setSavedLoginError(runtime, "saved_login_account_changed");
      saveState(runtime.state);
      return;
    }
    const snapshot = await captureBrowserSession(runtime.context, runtime.page);
    if (local.status !== "verified") return;
    const afterIdentity = await browserAccountFingerprint(runtime);
    if (!afterIdentity) { setSavedLoginError(runtime, "saved_login_identity_unavailable"); return; }
    if (afterIdentity !== local.account_fingerprint) {
      local.status = "external_login";
      setSavedLoginError(runtime, "saved_login_account_changed");
      saveState(runtime.state);
      return;
    }
    const envelope = encryptSessionEnvelope(snapshot, TOKEN);
    const publicationId = randomUUID();
    local.pending_publication_id = publicationId;
    saveState(runtime.state);
    const result = await apiRequest("POST", "/login-profile", { ...identity,
      profile_id: local.profile_id, binding_id: local.binding_id, generation: local.generation,
      expected_revision: local.source_revision, publication_id: publicationId,
      format_version: SESSION_FORMAT_VERSION, sealed_blob_base64: envelope.toString("base64") }, false);
    if (result.revision === publicationId && result.generation === local.generation) {
      local.source_revision = publicationId;
      local.pending_publication_id = null;
      setSavedLoginError(runtime, null);
      saveState(runtime.state);
    }
  } catch (error) {
    if (error?.status === 404) { setSavedLoginError(runtime, null); return; }
    setSavedLoginError(runtime, `saved_login_${stableErrorCode(error)}`);
  }
}

function setSavedLoginError(runtime, code) {
  if ((runtime.state.saved_login_error || null) === code) return;
  runtime.state.saved_login_error = code;
  saveState(runtime.state);
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
    loggedOutNoticeAt: 0,
    lastPresenceAt: 0,
    health: { http: 0, cdp: 0, tab: 0 },
  };
  log(`starting worker=${LABEL} version=${SCRIPT_VERSION}`);
  await recoverChrome(runtime);

  for (;;) {
    try {
      if (Date.now() - runtime.lastPresenceAt >= PRESENCE_MS) await heartbeat(runtime);
      if (await processPendingCommand(runtime)) process.exit(75);
      await processSavedLogin(runtime);
      if (runtime.state.draining && !runtime.state.current_task) {
        await sleep(POLL_MS);
        continue;
      }
      const handsOff =
        !runtime.state.current_task &&
        shouldLeaveTabAlone({
          url: runtime.page?.url(),
          loggedIn: runtime.loggedIn,
          pageOpen: Boolean(runtime.page && !runtime.page.isClosed()),
        });
      if (handsOff) {
        if (!runtime.loggedOutNoticeAt || Date.now() - runtime.loggedOutNoticeAt > 5 * 60 * 1000) {
          runtime.loggedOutNoticeAt = Date.now();
          log(
            "ChatGPT is not logged in; leaving the tab untouched and not claiming tasks " +
              "(log in on this screen or run: nyxid oracle login <pool>)"
          );
        }
        await sleep(POLL_MS);
        continue;
      }
      runtime.loggedOutNoticeAt = 0;
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
          !page.url().startsWith(response.required_project_url) &&
          !shouldLeaveTabAlone({ url: page.url(), loggedIn: runtime.loggedIn })
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

// Compare real paths: macOS temp dirs (/var -> /private/var) and other
// symlinked install locations make a plain path comparison fail, which
// silently skipped main() (and login capture) with exit code 0.
function isMainModule() {
  if (!process.argv[1]) return false;
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return resolve(process.argv[1]) === fileURLToPath(import.meta.url);
  }
}
const isMain = isMainModule();
if (isMain) {
  main().catch((error) => {
    console.error(`fatal: ${stableErrorCode(error)}`);
    process.exit(1);
  });
}
