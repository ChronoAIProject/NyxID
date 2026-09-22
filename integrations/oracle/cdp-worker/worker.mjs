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
  writeFileSync, realpathSync, existsSync, readdirSync, unlinkSync } from "node:fs";
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
const STABLE_INTERVAL_MS = Math.max(100, Math.min(60000,
  Number(process.env.NYXID_STABLE_INTERVAL_MS) || 8000));
const MAX_WAIT_MS = Number(process.env.NYXID_MAX_WAIT_MS || 2 * 60 * 60 * 1000); // 2h
// Wedge guard: if ChatGPT has clearly stopped (not generating) yet produced
// nothing extractable after this long, fail the task fast and free the slot
// instead of spinning to MAX_WAIT_MS. Mirrors the userscript's
// NO_OUTPUT_IDLE_TIMEOUT (420s).
const NO_OUTPUT_IDLE_MS = Number(process.env.NYXID_NO_OUTPUT_IDLE_MS || 7 * 60 * 1000);
export function usageCooldownConfig(value) {
  if (value === undefined) return { milliseconds: 900000, invalid: false };
  const seconds = Number(value);
  const invalid = !Number.isFinite(seconds) || seconds < 1;
  return { milliseconds: invalid ? 900000 : Math.min(86400, seconds) * 1000, invalid };
}
const USAGE_COOLDOWN = usageCooldownConfig(process.env.NYXID_ORACLE_USAGE_COOLDOWN_SECS);
const USAGE_COOLDOWN_MS = USAGE_COOLDOWN.milliseconds;
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

// ChatGPT's Temporary Chat: not saved to history, no memory, and a fresh
// document every time. With NYXID_ORACLE_TEMPORARY_CHAT=1 single-shot prompts
// run there so no draft, transcript or model state from an earlier task can
// bleed into the next one (a stale oversized draft is what stranded a
// 15-worker pool on 2026-09-21). Off by default: a Temporary Chat has no
// /c/<id> URL, so `nyxid oracle attach` cannot pick a single-shot answer up
// later. Session turns, follow-ups and project-pinned pools always keep
// persistent chats because their conversation URL must stay reachable.
export const TEMPORARY_CHAT_URL = "https://chatgpt.com/?temporary-chat=true";
const TEMPORARY_CHAT_ENABLED = process.env.NYXID_ORACLE_TEMPORARY_CHAT === "1";
const TEMPORARY_CHAT_ONBOARDING_SELECTOR = '[data-testid="modal-temporary-chat-onboarding"]';

export function isTemporaryChatUrl(url) {
  try {
    return new URL(url || "").searchParams.get("temporary-chat") === "true";
  } catch {
    return false;
  }
}

export function promptUsesTemporaryChat(task, enabled = TEMPORARY_CHAT_ENABLED) {
  return !!enabled && !task?.is_followup && !task?.conversation_id && !task?.required_project_url;
}

export function choosePromptNavigation({
  recovering,
  phase = "claimed",
  isFollowup,
  currentUrl,
  persistedUrl,
  taskConversationUrl,
  requiredProjectUrl,
  temporaryChat = false,
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
  // A Temporary Chat never exposes a /c/<id> URL, so after a send the live tab
  // is the only place the conversation exists. Keep it for post-send recovery
  // (the transcript check decides whether the sent prompt is really there);
  // any navigation would open an empty chat and lose the answer.
  if (recovering && !preSend && isTemporaryChatUrl(currentUrl) && !convId(currentUrl) &&
      isTemporaryChatUrl(persistedUrl) && !persistedConversationId && !taskConversationId) {
    return { error: null, target: null };
  }
  if ((isFollowup || recovering) && resumeUrl) {
    return {
      error: null,
      target:
        !resumeConversationId || currentConversationId !== resumeConversationId ? resumeUrl : null,
    };
  }
  if (requiredProjectUrl) {
    return { error: null, target: onConvPage || !currentUrl.startsWith(requiredProjectUrl) ? requiredProjectUrl : null };
  }
  // Every fresh Temporary Chat prompt reloads the surface: the URL does not
  // change after a turn, so it cannot prove the document is still pristine.
  if (temporaryChat) return { error: null, target: TEMPORARY_CHAT_URL };
  // A persistent prompt must never reuse a Temporary Chat surface left behind
  // by an earlier task: that conversation would vanish with the tab.
  const base = "https://chatgpt.com/";
  return { error: null, target: onConvPage || isTemporaryChatUrl(currentUrl) || !currentUrl.startsWith(base) ? base : null };
}

export function taskRecoveryDecision({
  kind,
  phase,
  failureCount,
  shapeFailures = 0,
  maxFailures = MAX_TASK_RECOVERY_FAILURES,
  relaunchEvery = MAX_CDP_FAILURES_BEFORE_RELAUNCH,
}) {
  const preSend = ["claimed", "page_ready", "ready_to_send"].includes(phase || "claimed");
  if (failureCount >= maxFailures || (preSend && shapeFailures >= 2)) {
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
    forceRelaunch: (preSend && shapeFailures > 0) || (relaunchEvery > 0 && failureCount % relaunchEvery === 0),
  };
}

export function failureDetail(code, phase) {
  const safeCode = /^[a-z0-9_]{1,64}$/.test(code || "") ? code : "worker_error";
  return /^[a-z0-9_]{1,40}$/.test(phase || "") ? `${safeCode}@${phase}` : safeCode;
}

export function cooldownRemaining(until, now = Date.now()) {
  return Math.max(0, Math.min(86400, Math.ceil(((Number(until) || 0) - now) / 1000)));
}

// Text is inspected only inside scoped UI banners; the result is a fixed code.
export function classifyChatGptError(text) {
  if (/reached.{0,60}(limit|cap)|usage limit|message cap|too many requests|达到.{0,20}(上限|限制)|已达.{0,20}上限/i.test(text || "")) return "usage_limit_reached";
  if (/model.{0,40}(unavailable|not available)|模型.{0,20}(不可用|无法使用)/i.test(text || "")) return "model_unavailable";
  if (/message.{0,40}too long|too long.{0,80}shorter|消息.{0,12}(太长|过长)|内容.{0,12}(太长|过长)/i.test(text || "")) return "prompt_too_long";
  if (/something went wrong|network error|error generating|unable to (generate|load)|出错了|发生错误|网络错误/i.test(text || "")) return "chatgpt_error_response";
  return null;
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

export function stableErrorCode(error) {
  if (error?.code && /^[a-z0-9_]{1,64}$/.test(error.code)) return error.code;
  if (error?.code && /^[A-Z][A-Z0-9_]{1,63}$/.test(error.code)) return error.code.toLowerCase();
  if (error?.status) return `http_${error.status}`;
  const message = String(error?.message || "").toLowerCase();
  if (/target crashed|page crashed|page crash/.test(message)) return "page_crashed";
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
const DOM_CORE_VERSION = 5;
const DOM_CORE = `
window.__nyx = (function () {
  const artifactFileId = ${artifactFileId.toString()};
  const isTrustedArtifactUrl = ${isTrustedArtifactUrl.toString()};
  const sanitizeArtifactName = ${sanitizeArtifactName.toString()};
  const classifyArtifactLink = ${classifyArtifactLink.toString()};
  const compactModelLabel = ${compactModelLabel.toString()};

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
    const { input } = discoverControls();
    const region = input?.closest("form") || input?.parentElement;
    const stop = region?.querySelector("button[data-testid='stop-button'], button[aria-label='Stop generating'], button[aria-label='Stop streaming'], button[aria-label='停止生成']");
    if (stop && pickerElementVisible(stop) && !stop.closest('[data-message-author-role]')) return true;
    const turn = latestAssistantTurn();
    if (!turn) return false;
    // Explicit live state only. Collapsed reasoning and persistent Pro pills
    // are not evidence that a response is still streaming.
    return [turn, ...turn.querySelectorAll('[data-is-streaming="true"], [data-state="streaming"], [aria-busy="true"], .result-streaming')]
      .some(el => pickerElementVisible(el) && (el.matches('[data-is-streaming="true"], [data-state="streaming"], [aria-busy="true"], .result-streaming')));
  }

  const classifyChatGptError = ${classifyChatGptError.toString()};
  function errorCode() {
    const latest = latestAssistantTurn();
    const banners = [...document.querySelectorAll('[role="alert"], [data-testid="error-message"], [data-testid="conversation-error"]')]
      .filter(el => pickerElementVisible(el) && (!el.closest('[data-message-author-role]') || latest?.contains(el)));
    const composer = discoverControls().input?.closest('form');
    if (composer) banners.push(...composer.querySelectorAll('[role="status"]'));
    for (const banner of banners) {
      const code = classifyChatGptError(banner.innerText);
      if (code) return code;
    }
    // A normal Regenerate action is not a failure; an explicit retry affordance is.
    if (latest && [...latest.querySelectorAll('button[data-testid="retry-button"]')].some(pickerElementVisible)) return 'chatgpt_error_response';
    return null;
  }

  function discoverControls() {
    document.querySelectorAll('[data-nyx-composer], [data-nyx-send]').forEach(el => {
      el.removeAttribute('data-nyx-composer'); el.removeAttribute('data-nyx-send');
    });
    const valid = el => pickerElementVisible(el) && !el.closest('[data-message-author-role], [role="dialog"]') && (el.tagName === 'TEXTAREA' || el.isContentEditable);
    const exact = [...document.querySelectorAll('#prompt-textarea, [data-testid="prompt-textarea"], [contenteditable="true"][role="textbox"]')].filter(valid);
    const candidates = exact.length ? exact : [...document.querySelectorAll('main form textarea, main form [contenteditable="true"], main [role="textbox"]')].filter(valid);
    const input = candidates.length === 1 ? candidates[0] : null;
    if (!input) return { input: null, send: null };
    input.setAttribute('data-nyx-composer', '');
    let region = input.closest('form') || input.parentElement;
    const selector = 'button[data-testid="send-button"], button[aria-label="Send prompt"], button[aria-label="发送提示"], button[aria-label="Send message"], button[aria-label="发送消息"]';
    while (region && region !== document.body && !region.querySelector(selector) && !input.closest('form')) region = region.parentElement;
    const known = [...(region?.querySelectorAll(selector) || [])].filter(pickerElementVisible);
    const fallback = input.closest('form') ? [...input.closest('form').querySelectorAll('button[type="submit"]')].filter(pickerElementVisible) : [];
    const sends = known.length ? known : fallback;
    const send = sends.length === 1 ? sends[0] : null;
    if (send) send.setAttribute('data-nyx-send', '');
    return { input, send };
  }

  function structuralProbe() {
    const { input, send } = discoverControls();
    const roles = [...document.querySelectorAll('[data-message-author-role]')];
    const role = roles.at(-1)?.getAttribute('data-message-author-role');
    const login = [...document.querySelectorAll('a,button')].some(el => /^(log in|sign up|登录|注册)$/i.test((el.textContent || '').trim()));
    const account = document.querySelector('[data-testid="profile-button"], [data-testid="model-switcher-dropdown-button"]') ||
      [...document.querySelectorAll('header button[aria-haspopup]')].find(el => pickerElementVisible(el) && /^(chatgpt|gpt)[\\s_-]*[0-9]{1,3}(?:[._][0-9]{1,3})?(?=$|[\\s_-])/i.test((el.innerText || '').trim()));
    return { composer_found: !!input, send_found: !!send, pill_found: !!document.querySelector('button.__composer-pill'),
      helper_installed: !!window.__nyx, logged_in: !login && !!(input || account),
      url_host: ['chatgpt.com', 'chat.openai.com'].includes(location.hostname) ? location.hostname : 'other',
      error_banner: !!errorCode(), latest_turn_role: ['assistant', 'user'].includes(role) ? role : 'none', turns: roles.length };
  }

  function assistantCount() {
    return document.querySelectorAll("[data-message-author-role='assistant']").length;
  }

  // Structure only, never content: what the page looked like when a task
  // failed. Lengths and roles stand in for text; test ids and ARIA state
  // stand in for labels. The composer draft is reported by length alone.
  function diagnosticSummary() {
    const { input, send } = discoverControls();
    const visible = (el) => pickerElementVisible(el);
    const describe = (el) => ({ tag: el.tagName, testid: el.getAttribute('data-testid'), role: el.getAttribute('role'),
      aria_label_length: (el.getAttribute('aria-label') || '').length, text_length: (el.innerText || '').trim().length });
    const pill = document.querySelector('button.__composer-pill');
    const draft = input ? String(input.value ?? input.innerText ?? '') : '';
    const latest = latestAssistantTurn();
    return {
      viewport: { width: innerWidth, height: innerHeight, visibility: document.visibilityState, ready: document.readyState },
      body_pointer_events: getComputedStyle(document.body).pointerEvents,
      composer: input ? { ...describe(input), editable: !!input.isContentEditable || input.tagName === 'TEXTAREA', draft_length: draft.length,
        rect: (() => { const r = input.getBoundingClientRect(); return { top: Math.round(r.top), height: Math.round(r.height) }; })() } : null,
      send: send ? { ...describe(send), disabled: !!send.disabled } : null,
      pill: pill ? { text_length: (pill.innerText || '').trim().length, expanded: pill.getAttribute('aria-expanded'), state: pill.getAttribute('data-state') } : null,
      dialogs: [...document.querySelectorAll('dialog[open], [role="dialog"]')].filter(visible).map((el) => ({
        ...describe(el), modal_testid: el.querySelector('[data-testid]')?.getAttribute('data-testid') || null,
        buttons: [...el.querySelectorAll('button')].filter(visible).length })),
      menus: [...document.querySelectorAll('[role="menu"], [role="listbox"]')].filter(visible).length,
      alerts: [...document.querySelectorAll('[role="alert"], [role="status"]')].filter(visible).map((el) => ({ ...describe(el), code: classifyChatGptError(el.innerText) })),
      error_code: errorCode(),
      generating: isStillGenerating(),
      turns: { total: document.querySelectorAll('[data-message-author-role]').length, assistant: assistantCount(),
        latest_role: latest ? 'assistant' : 'none', latest_length: latest ? (latest.innerText || '').length : 0 },
      url: { host: location.hostname, path: location.pathname, temporary: new URLSearchParams(location.search).get('temporary-chat') === 'true' },
    };
  }

  function latestAssistantTurn() {
    const main = document.querySelector("main");
    if (!main) return null;
    const turns = main.querySelectorAll('[data-testid^="conversation-turn"]');
    if (turns.length) {
      const scope = turns[turns.length - 1];
      return scope.querySelector("[data-message-author-role='user']") ? null : scope;
    }
    const els = main.querySelectorAll("[data-message-author-role]");
    const last = els[els.length - 1];
    return last?.getAttribute('data-message-author-role') === 'assistant' ? last : null;
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
    const scope = latestAssistantTurn();
    if (!scope) return "";
    const assistant = scope.matches("[data-message-author-role='assistant']") ? scope : scope.querySelector("[data-message-author-role='assistant']");
    return assistant ? cleanText(extractTextWithMath(assistant)) : "";
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
  let parentModelMenus = null;
  function pickerElementVisible(el) {
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none";
  }
  function visibleModelMenus() {
    return [...document.querySelectorAll('[role="menu"], [role="listbox"]')].filter(pickerElementVisible);
  }
  function beginModelPicker(id, nested = false) {
    parentModelMenus = nested && id === modelPickerId ? preexistingModelMenus : null;
    modelPickerId = id;
    preexistingModelMenus = new WeakSet(visibleModelMenus());
    return true;
  }
  function finishNestedModelPicker(id) {
    if (id === modelPickerId && parentModelMenus) preexistingModelMenus = parentModelMenus;
    parentModelMenus = null;
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

  return { version: ${DOM_CORE_VERSION}, discoverControls, structuralProbe, diagnosticSummary, errorCode, isStillGenerating, assistantCount, extractResponse, extractImages, extractFiles, extractTranscript, extractTranscriptKeys, scrollContainer, extractTextWithMath, cleanText,
    beginModelPicker, finishNestedModelPicker, modelPickerMenus, modelPickerItems, modelPickerTrigger, modelPickerItem, compactModelLabel };
})();
`;

// Wait for the page to go quiet instead of sleeping a fixed interval: resolve
// once no DOM mutation has landed for `quietMs`, or after `maxMs` regardless.
// A fixed sleep is either too short on a loaded page or wasted on a fast one;
// this returns as soon as React's last batch settles. Falls back to a short
// sleep when the page cannot be evaluated (navigating, crashed).
export async function settleDom(page, { quietMs = 150, maxMs = 2500 } = {}) {
  const started = Date.now();
  try {
    const waited = await Promise.race([
      page.evaluate(({ quiet, max }) => new Promise((resolveSettle) => {
        let timer = null;
        const done = () => { observer.disconnect(); clearTimeout(timer); clearTimeout(cap); resolveSettle(true); };
        const bump = () => { clearTimeout(timer); timer = setTimeout(done, quiet); };
        const observer = new MutationObserver(bump);
        observer.observe(document.documentElement, { subtree: true, childList: true, characterData: true, attributes: true });
        const cap = setTimeout(done, max);
        bump();
      }), { quiet: quietMs, max: maxMs }),
      sleep(maxMs + 1000).then(() => false),
    ]);
    return { settled: waited === true, ms: Date.now() - started };
  } catch (error) {
    if (["page_crashed", "cdp_disconnected"].includes(stableErrorCode(error))) throw error;
    await sleep(Math.min(maxMs, 500));
    return { settled: false, ms: Date.now() - started };
  }
}

// Poll for the composer to hydrate, bounded; a missing composer is reported
// by the caller's own composer_not_found path, never here.
async function waitForComposer(page, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      if (await page.evaluate(() => !!window.__nyx?.discoverControls().input)) return true;
    } catch (error) {
      if (["page_crashed", "cdp_disconnected"].includes(stableErrorCode(error))) throw error;
    }
    await sleep(100);
  }
  return false;
}

const initializedPages = new WeakSet();
export async function installDomCore(page) {
  if (!initializedPages.has(page)) {
    await page.addInitScript({ content: DOM_CORE });
    initializedPages.add(page);
  }
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      if (await page.evaluate(version => window.__nyx?.version === version, DOM_CORE_VERSION)) return;
      await page.evaluate(DOM_CORE);
      if (await page.evaluate(version => window.__nyx?.version === version, DOM_CORE_VERSION)) return;
    } catch (error) {
      if (['page_crashed', 'cdp_disconnected'].includes(stableErrorCode(error))) throw error;
    }
    await sleep(100);
  }
  throw Object.assign(new Error('dom_core_unavailable'), { code: 'dom_core_unavailable' });
}

// ── Failure diagnostics ──────────────────────────────────────────────────
// On every task failure the worker writes a structural snapshot of the page
// next to its state file, so a failure can be explained after the fact
// without sudo on the host or a repro. JSON always; a PNG only when
// NYXID_ORACLE_DIAGNOSTIC_SCREENSHOTS=1, because a screenshot shows content.
// The newest NYXID_ORACLE_DIAGNOSTICS_KEEP snapshots are retained.
const DIAGNOSTICS_DIR = process.env.NYXID_ORACLE_DIAGNOSTICS_DIR || resolve(dirname(STATE_FILE), "diagnostics");
const DIAGNOSTICS_KEEP = Math.max(0, Math.min(500, Number(process.env.NYXID_ORACLE_DIAGNOSTICS_KEEP) || 20));
const DIAGNOSTIC_SCREENSHOTS = process.env.NYXID_ORACLE_DIAGNOSTIC_SCREENSHOTS === "1";
const DIAGNOSTIC_CAPTURE_MS = 5000;

export function diagnosticFileName(now, taskId, code) {
  const stamp = new Date(now).toISOString().replace(/[:.]/g, "-").replace(/Z$/, "Z");
  const task = String(taskId || "task").replace(/[^a-z0-9]/gi, "").slice(0, 8) || "task";
  const safeCode = /^[a-z0-9_]{1,64}$/.test(code || "") ? code : "worker_error";
  return `${stamp}-${task}-${safeCode}`;
}

// Snapshot base names sort chronologically; return the ones beyond `keep`.
export function diagnosticsToPrune(names, keep) {
  const bases = [...new Set((names || []).map((name) => String(name).replace(/\.(json|png)$/, "")))]
    .filter((base) => /^\d{4}-\d{2}-\d{2}T/.test(base)).sort();
  const stale = bases.slice(0, Math.max(0, bases.length - keep));
  return (names || []).filter((name) => stale.includes(String(name).replace(/\.(json|png)$/, "")));
}

export async function writeDiagnosticSnapshot(runtime, task, code, detail) {
  const page = runtime?.page;
  const base = diagnosticFileName(Date.now(), task?.task_id, code);
  try {
    mkdirSync(DIAGNOSTICS_DIR, { recursive: true, mode: 0o700 });
    const capture = async (read) => Promise.race([
      read().catch((error) => ({ unavailable: stableErrorCode(error) })),
      new Promise((resolveTimeout) => setTimeout(() => resolveTimeout({ unavailable: "capture_timeout" }), DIAGNOSTIC_CAPTURE_MS)),
    ]);
    const live = page && !page.isClosed();
    const snapshot = {
      at: new Date().toISOString(), worker: LABEL, script_version: SCRIPT_VERSION,
      task_id: task?.task_id || null, kind: task?.kind || null, code, detail,
      phase: runtime?.state?.current_task?.phase || null, last_phase: runtime?.state?.current_task?.last_phase || null,
      recovery_failures: runtime?.state?.current_task?.recovery_failures || 0,
      prompt_length: typeof task?.prompt === "string" ? task.prompt.length : null,
      model: task?.model || null,
      observed_model_switcher: runtime?.state?.current_task?.observed_model_switcher || null,
      observed_model_effort: runtime?.state?.current_task?.observed_model_effort || null,
      url: live ? page.url() : null,
      chrome_alive: !!runtime?.chromeAlive, logged_in: runtime?.loggedIn ?? null,
      probe: live ? await capture(() => failureProbe(page)) : { unavailable: "no_page" },
      summary: live ? await capture(async () => { await installDomCore(page); return page.evaluate(() => window.__nyx?.diagnosticSummary()); }) : { unavailable: "no_page" },
    };
    const jsonPath = resolve(DIAGNOSTICS_DIR, `${base}.json`);
    writeFileSync(jsonPath, JSON.stringify(snapshot, null, 1), { mode: 0o600 });
    if (DIAGNOSTIC_SCREENSHOTS && live) {
      await capture(() => page.screenshot({ path: resolve(DIAGNOSTICS_DIR, `${base}.png`), timeout: DIAGNOSTIC_CAPTURE_MS }));
    }
    for (const stale of diagnosticsToPrune(readdirSync(DIAGNOSTICS_DIR), DIAGNOSTICS_KEEP)) {
      try { unlinkSync(resolve(DIAGNOSTICS_DIR, stale)); } catch {}
    }
    log(`diagnostic_snapshot file=${base}.json`);
    return jsonPath;
  } catch (error) {
    log(`diagnostic_snapshot failed (${stableErrorCode(error)})`);
    return null;
  }
}

async function failureProbe(page) {
  try {
    await installDomCore(page);
    return await page.evaluate(() => window.__nyx?.structuralProbe());
  } catch {
    return { composer_found: false, send_found: false, pill_found: false, helper_installed: false,
      logged_in: false, url_host: 'unavailable', error_banner: false, latest_turn_role: 'none', turns: 0 };
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
    await installDomCore(page);
    return await page.evaluate(() => window.__nyx?.structuralProbe().logged_in === true);
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
  runtime.page.on("crash", () => { runtime.pageCrashed = true; });
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

export async function replaceCrashedPage(runtime, targetUrl) {
  const stale = runtime.page;
  runtime.page = await runtime.context.newPage();
  runtime.page.on('crash', () => { runtime.pageCrashed = true; });
  runtime.pageCrashed = false;
  await stale?.close().catch(() => {});
  await runtime.page.goto(targetUrl || 'https://chatgpt.com/', { waitUntil: 'domcontentloaded', timeout: 60000 });
  await installDomCore(runtime.page);
  return runtime.page;
}

async function ensureChatPage(runtime, targetUrl) {
  if (runtime.pageCrashed && runtime.context) await replaceCrashedPage(runtime, targetUrl);
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
    await installDomCore(runtime.page);
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
// entries; "-pro" (the pool default `chatgpt-6-pro`) selects the Pro level.
// Chinese labels are kept so either localisation matches. The first entry is
// the canonical display name.
export function modelLevelTargets(label) {
  const raw = String(label || "").trim();
  if (!raw) return [];
  const lower = raw.toLowerCase();
  const compact = lower.replace(/^(chatgpt|openai)-/, "").replace(/[\s._-]+/g, "");
  // Pro plans may split Pro into "Pro Standard" and "Pro Extended" entries.
  // The canonical level stays "Pro" (verification and phase_detail use it);
  // the alias order decides which entry the exact pass prefers.
  if (/standard|标准/.test(lower)) return ["Pro", "Pro Standard", "Pro 标准", "标准"];
  if (/扩展|extended/.test(lower)) return ["Pro", "Pro Extended", "Pro 扩展", "扩展"];
  if (/\bpro\b|pro$|专业/.test(lower) || compact.endsWith("pro")) {
    return ["Pro", "Pro Extended", "Pro 扩展", "扩展"];
  }
  if (/extra[\s._-]*high|ultra|超高/.test(lower)) return ["Extra High", "超高"];
  if (/\bhigh\b|高级|advanced/.test(lower)) return ["High", "高级"];
  if (/medium|balanced|均衡/.test(lower)) return ["Medium", "均衡"];
  if (/instant|fast|极速/.test(lower)) return ["Instant", "极速"];
  return [raw];
}

function normalizeMenuText(value) {
  return String(value || "").toLowerCase().replace(/[\s·:()|/—._-]+/g, "");
}

// Exact pass first so "High" never selects "Extra High"; fuzzy pass second.
export function modelItemMatches(itemText, targets, exact) {
  const candidate = normalizeMenuText(String(itemText || "").trim().split(/\r?\n/)[0]);
  if (!candidate) return false;
  if (targets?.[0] === "Pro" && !pillShowsLevel(itemText, targets)) return false;
  const wanted = (targets || []).map(normalizeMenuText).filter(Boolean);
  if (!exact && MODEL_LEVELS.some((aliases) => aliases[0] === targets?.[0]) &&
      detectPillLevel(itemText) !== targets[0]) return false;
  return exact
    ? wanted.some((w) => candidate === w)
    : wanted.some((w) => candidate.includes(w) || w.includes(candidate));
}

const MODEL_SELECT_TIMEOUT_MS = Math.max(1, Math.min(25000,
  Number(process.env.NYXID_MODEL_SELECT_TIMEOUT_MS) || 25000));
// Selection is bounded by silence, not by the clock: every observed step
// (menu opened, slider moved, entry clicked, read-back confirmed) restarts the
// MODEL_SELECT_TIMEOUT_MS window, up to this hard ceiling. A slow page that
// keeps making progress finishes; a stuck one still dies within one window.
const MODEL_SELECT_MAX_MS = Math.max(MODEL_SELECT_TIMEOUT_MS, Math.min(180000,
  Number(process.env.NYXID_MODEL_SELECT_MAX_MS) || 90000));
const LOG_PICKER_LABELS = process.env.NYXID_ORACLE_LOG_PICKER_LABELS === "1";
// Clamp a composer bounding rect to the region actually on screen and return
// the centre of what remains, or null when nothing is visible. Intersect the
// rect with the viewport, then with each scroll/clip ancestor (clips: entries
// of { x, y, left, right, top, bottom } where x/y say the axis is clipped).
// A very long draft can push the composer's geometric centre tens of thousands
// of pixels above the viewport, where elementFromPoint returns null and the
// composer is wrongly judged obstructed (composer_unobstructed_failed).
// NOTE: keep this in sync with the inline copy inside ensureComposerUnobstructed
// (same maths, separate runtime), mirroring the fileMime split noted below.
export function composerVisibleHitPoint(rect, clips = [], viewport = {}) {
  if (!rect) return null;
  let left = Math.max(0, rect.left);
  let right = Math.min(viewport.width, rect.right);
  let top = Math.max(0, rect.top);
  let bottom = Math.min(viewport.height, rect.bottom);
  for (const clip of clips) {
    if (clip.x) { left = Math.max(left, clip.left); right = Math.min(right, clip.right); }
    if (clip.y) { top = Math.max(top, clip.top); bottom = Math.min(bottom, clip.bottom); }
  }
  if (!(right > left && bottom > top)) return null;
  return { x: (left + right) / 2, y: (top + bottom) / 2 };
}

// A composer holds a draft worth clearing when it contains any non-whitespace.
// Used by clearComposerDraft to skip the clear on an already-empty composer.
export function composerHasDraft(text) {
  return typeof text === "string" && text.trim().length > 0;
}

// A draft this long defeats the ordinary clear. fill("") has to drive a
// select-all and delete through React over tens of thousands of characters,
// which does not finish inside PRE_SEND_ACTION_MS; the clear is best-effort,
// so the timeout is swallowed and the draft survives. Model selection then
// runs against the heavy composer and dies as operation_timeout@selecting_model
// - and because the draft outlives a browser relaunch, every worker that picks
// the task up is stranded the same way. Observed 2026-09-21: one 83,046
// character prompt walked through a 15-worker pool, disabling each tab it
// touched until the task was cancelled by hand.
export const COMPOSER_FAST_CLEAR_CHARS = 2000;

export function draftNeedsFastClear(length) {
  return Number.isFinite(length) && length >= COMPOSER_FAST_CLEAR_CHARS;
}

export const PRE_SEND_ACTION_MS = 5000;

// Typing the prompt must not share the flat pre-send allowance. fill() drives
// the whole prompt through the composer's React handlers, and five seconds is
// ample for a chat message but not for a long one. When it overruns, the error
// carries "timeout", stableErrorCode maps it to operation_timeout, and the last
// acked phase is still selecting_model - so a task that selected its model
// perfectly reports operation_timeout@selecting_model, with the partly typed
// prompt left in the composer. That leftover then strands the tab for the next
// pickup. Observed 2026-09-22: task 9a6d7697 carried a 50,432 character prompt,
// logged "already_selected selected=Pro", then died ~8s later on every worker it
// reached, leaving an identical draft on four machines.
// Scale the allowance with the prompt and keep a ceiling so a pathological one
// still fails promptly rather than hanging the attempt.
export const PROMPT_FILL_CHARS_PER_MS = 5;
export const PROMPT_FILL_MAX_MS = 60000;

export function promptFillTimeout(length) {
  const n = Number(length);
  const scaled = Number.isFinite(n) && n > 0 ? Math.ceil(n / PROMPT_FILL_CHARS_PER_MS) : 0;
  return Math.min(PROMPT_FILL_MAX_MS, Math.max(PRE_SEND_ACTION_MS, scaled));
}

// ChatGPT refuses an over-long message server-side: the conversation POST
// returns HTTP 413 (message_length_exceeds_limit) and the page shows "The
// message you submitted was too long". By then the worker has spent the whole
// fill allowance typing it and, on an overrun, left the draft behind for the
// next pickup. Refuse a prompt that cannot be delivered before touching the
// composer, and name the rejection precisely when ChatGPT refuses one that
// was typed. The default ceiling is what the fill allowance can type at all
// (PROMPT_FILL_MAX_MS at PROMPT_FILL_CHARS_PER_MS); a pool that has proven a
// higher limit can raise NYXID_MAX_PROMPT_CHARS, and 0 disables the check.
export const PROMPT_MAX_CHARS = (() => {
  const configured = Number(process.env.NYXID_MAX_PROMPT_CHARS);
  if (process.env.NYXID_MAX_PROMPT_CHARS !== undefined && Number.isFinite(configured) && configured >= 0) return configured;
  return PROMPT_FILL_MAX_MS * PROMPT_FILL_CHARS_PER_MS;
})();

export function promptExceedsLimit(length, max = PROMPT_MAX_CHARS) {
  const n = Number(length);
  return Number.isFinite(n) && Number.isFinite(max) && max > 0 && n > max;
}

// Only the page's own conversation POST from the main frame can classify this
// send; a background endpoint, another tab or an old response cannot.
export function classifySubmissionResponse({ method, url, status }) {
  if (method !== "POST") return null;
  // Observed 2026-09-22: the page POSTs .../f/conversation/prepare, then
  // .../f/conversation. Either may refuse the message.
  if (!/^https:\/\/(chatgpt\.com|chat\.openai\.com)\/backend-api\/(f\/)?conversation(\/prepare)?(\?|$)/.test(url || "")) return null;
  if (status === 413) return "prompt_too_long";
  return null;
}

function observeSubmissionRejection(page) {
  const observer = { code: null, stop: () => {} };
  const onResponse = (response) => {
    try {
      const request = response.request();
      if (request.frame() !== page.mainFrame()) return;
      const code = classifySubmissionResponse({ method: request.method(), url: request.url(), status: response.status() });
      if (code && !observer.code) observer.code = code;
    } catch {
      // Diagnostics only; never let an observer error touch the task flow.
    }
  };
  page.on("response", onResponse);
  observer.stop = () => { try { page.off("response", onResponse); } catch {} };
  return observer;
}
const COMPOSER_SELECTOR = "[data-nyx-composer]";
const SEND_SELECTOR = "[data-nyx-send]";
const PILL_SELECTOR = 'button.__composer-pill[aria-haspopup="menu"]:visible:not([data-nyx-switcher])';
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
  const canonical = (value) => String(value || '')
    .replace(/^(?:(?:chatgpt|gpt)[\s._-]*)?\d+(?:\.\d+)*[\s._-]*/i, '')
    .toLowerCase().replace(/[·:()|/—._-]+/g, ' ').replace(/\s+/g, ' ').trim();
  // Accept only a vocabulary of level tokens, never prose containing "Pro".
  const classify = (label) => {
    if (/^(?:thinking|pro|专业|extended|standard|扩展|标准)(?: (?:thinking|pro|专业|extended|standard|扩展|标准))*$/.test(label) &&
        /(?:^| )(?:pro|专业|extended|standard|扩展|标准)(?: |$)/.test(label)) return 'Pro';
    return MODEL_LEVELS.find(aliases => aliases.some(alias => alias.toLowerCase() === label))?.[0] || null;
  };
  const trimmed = String(text || '').trim();
  // First line only, as before: a menu entry's second line is usually a
  // description, and folding it in would wreck an otherwise exact match.
  const firstLine = classify(canonical(trimmed.split(/\r?\n/)[0]));
  if (firstLine) return firstLine;
  // Only when that yields nothing, retry with the newlines flattened. The
  // composer pill renders the family and the level as separate text nodes
  // ("6\nPro"), so first-line-only reads "6", strips it as a version number,
  // and reports the live GPT-6 Pro pill as unrecognized.
  return trimmed.includes('\n') ? classify(canonical(trimmed.replace(/\s*\r?\n+\s*/g, ' '))) : null;
}

// Unrecognized composer controls can be explored but are not negative evidence.
export function effortSelectionMismatch({ observed, verified, recognizedLevels = false, recognizedObservation = false }, requested) {
  return (recognizedObservation && detectPillLevel(observed) === null) ||
    (detectPillLevel(observed) !== null && !pillShowsLevel(observed, modelLevelTargets(requested))) ||
    (recognizedLevels && !verified);
}

export function pillShowsLevel(pillText, targets) {
  pillText = String(pillText || "").trim().split(/\r?\n/)[0];
  const canonical = (targets || [])[0];
  if (!canonical || !pillText) return false;
  if (MODEL_LEVELS.some((aliases) => aliases[0] === canonical)) {
    if (detectPillLevel(pillText) !== canonical) return false;
    if (canonical === 'Pro') {
      const wantedStandard = targets.includes('Pro Standard');
      if (/standard|标准/i.test(pillText)) return wantedStandard;
      if (/extended|扩展/i.test(pillText)) return !wantedStandard;
    }
    return true;
  }
  return normalizeMenuText(pillText).includes(normalizeMenuText(canonical));
}

// Index in a snapshot of visible entries. Never commit an arbitrary first
// item, even if checked. A submenu may contain account actions, not levels.
export function chooseNestedLevelEntry(items, targets, allowChecked = true) {
  const recognized = (item) => detectPillLevel(item.text) !== null;
  // Canonical split-tier recognition also covers localized separator labels.
  if (targets?.[0] === 'Pro') {
    const expected = targets.includes('Pro Standard') ? 'pro_standard' : 'pro_extended';
    const tiers = items.map((item, index) => ({ ...item, index })).filter(item => effortMetadata(item.text) === expected);
    if (tiers.length) return (tiers.find(item => targets.some(target => modelItemMatches(item.text, [target], true))) || tiers[0]).index;
  }
  // Split tiers outrank generic Pro. Never fall back to a checked lower tier.
  const priority = targets?.[0] === 'Pro' ? [...targets.slice(1), targets[0]] : targets;
  for (const target of priority || []) {
    const index = items.findIndex((item) => recognized(item) && modelItemMatches(item.text, [target], true));
    if (index >= 0) return index;
  }
  const fuzzy = items.findIndex((item) => recognized(item) && modelItemMatches(item.text, targets, false));
  if (fuzzy >= 0) return fuzzy;
  return allowChecked ? items.findIndex((item) => recognized(item) && item.checked && modelItemMatches(item.text, targets, false)) : -1;
}

// Canonical observations deliberately discard raw UI labels.
export function switcherMetadata(text) {
  const label = String(text || '').trim().split(/\r?\n/)[0];
  if (!label) return 'absent';
  const match = /^(?:chatgpt|gpt)[\s_-]*([0-9]{1,3})(?:[._]([0-9]{1,3}))?(?:[\s_-]+(.+))?$/i.exec(label);
  if (!match) return 'unrecognized';
  const tier = match[3]?.trim();
  const pro = detectPillLevel(tier) === 'Pro';
  if (tier && !pro && !/^(?:auto|instant|thinking|medium|high|extra[ -]high|自动|极速|思考|均衡|高级|超高)$/i.test(tier)) return 'unrecognized';
  return `gpt_${Number(match[1])}${match[2] ? `_${Number(match[2])}` : ''}${pro ? '_pro' : ''}`;
}

// Family evidence from the picker's model-version radios ("Latest",
// "GPT-5.6 Sol", "GPT-5.5"). At every level except Pro the composer pill and
// the "Select model" row show only the level ("High"), so the checked radio
// is the only place the family can be read. "Latest" carries no number and
// is reported as gpt_latest.
export function familyFromModelRadios(items) {
  const checked = (items || []).find((item) => item?.checked);
  const label = String(checked?.text || '').trim().split(/\r?\n/)[0].trim();
  if (!label) return 'absent';
  if (/^(latest|最新)$/i.test(label)) return 'gpt_latest';
  const metadata = switcherMetadata(label.replace(/\s+(sol|thinking)$/i, ''));
  return metadata === 'unrecognized' ? 'absent' : metadata;
}

// Canonical metadata compared to a request. gpt_latest satisfies a request
// that names no minor version (chatgpt-6-high, chatgpt-6-pro) but never one
// pinned to an older release (chatgpt-5.5-high needs the GPT-5.5 radio).
export function switcherMetadataMatches(metadata, requested) {
  if (!metadata || ['absent', 'unrecognized'].includes(metadata)) return false;
  const request = String(requested || '').replace(/^openai-/, 'gpt-');
  if (metadata === 'gpt_latest') {
    const wanted = switcherMetadata(request);
    if (wanted === 'unrecognized') return modelLevelTargets(request).length > 0 && !/\d+[._]\d+/.test(request);
    return !/^gpt_\d+_\d+/.test(wanted);
  }
  const parse = (meta) => /^gpt_(\d+)(?:_(\d+))?(_pro)?$/.exec(meta);
  let target = switcherMetadata(request);
  if (target === 'unrecognized' && !/\d/.test(request) && modelLevelTargets(request)[0] === 'Pro') target = 'gpt_6_pro';
  const wanted = parse(target), observed = parse(metadata);
  return !!(wanted && observed && wanted[1] === observed[1] && (!wanted[2] || !observed[2] || wanted[2] === observed[2]));
}

export function switcherMatches(text, requested, familyOnly = false) {
  const request = String(requested || '').replace(/^openai-/, 'gpt-');
  let target = switcherMetadata(request);
  if (target === 'unrecognized' && !/\d/.test(request) && modelLevelTargets(request)[0] === 'Pro') target = 'gpt_6_pro';
  const parse = meta => /^gpt_(\d+)(?:_(\d+))?(_pro)?$/.exec(meta);
  const wanted = parse(target), observed = parse(switcherMetadata(text));
  return !!(wanted && observed && wanted[1] === observed[1] &&
    (!wanted[2] || !observed[2] || wanted[2] === observed[2]) &&
    (familyOnly || wanted[3] === observed[3]));
}

// Compact labels establish family/tier only, never reasoning effort or a
// bare family submenu. Require the entire label to contain known tokens.
export function compactModelLabel(text) {
  const label = String(text || '').trim().replace(/\s+/g, ' ');
  const match = /^([0-9]{1,3}(?:\.[0-9]{1,3})?) (pro|专业|auto|instant|thinking|medium|high|extra[ -]high|自动|极速|思考|均衡|高级|超高)$/i.exec(label);
  return match ? `GPT ${match[1]} ${match[2]}` : null;
}

export function chooseSwitcherEntry(items, requested, familyContext = null) {
  const candidates = items.map((item, index) => ({ ...item, text: compactModelLabel(item.text) || item.text, index }))
    .filter(item => switcherMatches(item.text, requested));
  const exact = candidates.find(item => normalizeMenuText(item.text).replace(/^chatgpt/, 'gpt') === normalizeMenuText(requested).replace(/^chatgpt/, 'gpt'));
  if (candidates.length) return (exact || candidates[0]).index;
  if (switcherMatches(familyContext, requested, true) && modelLevelTargets(requested)[0] === 'Pro') {
    return items.findIndex(item => /^(?:pro|专业)$/i.test(String(item.text || '').trim().split(/\r?\n/)[0].trim()));
  }
  return -1;
}

export function chooseSwitcherFamilyEntry(items, requested) {
  return items.findIndex(item => /^(?:chatgpt|gpt)[\s_-]*[0-9]{1,3}(?:[._][0-9]{1,3})?$/i.test(
    String(item.text || '').trim().split(/\r?\n/)[0]) && switcherMatches(item.text, requested, true));
}

export function effortMetadata(text) {
  text = String(text || '').trim().split(/\r?\n/)[0];
  if (!text) return 'absent';
  const level = detectPillLevel(text);
  if (level === 'Pro') {
    if (/extended|扩展/i.test(text)) return 'pro_extended';
    if (/standard|标准/i.test(text)) return 'pro_standard';
  }
  return level ? level.toLowerCase().replace(/ /g, '_') : 'unrecognized';
}

export async function readModelSwitcher(page, budget = interactionBudget(1000)) {
  const snapshot = await boundedRead(budget, timeout => page.locator('body').evaluate((body, { deadline, pickerId }) => {
    if (Date.now() >= deadline) return null;
    const visible = el => el.getBoundingClientRect().width > 0 && el.getBoundingClientRect().height > 0 && getComputedStyle(el).visibility !== 'hidden';
    body.querySelectorAll('[data-nyx-switcher]').forEach(el => el.removeAttribute('data-nyx-switcher'));
    const exact = [...body.querySelectorAll('button[data-testid="model-switcher-dropdown-button"]')].filter(visible);
    const fallback = [...body.querySelectorAll('header button[aria-haspopup="menu"], header button[aria-haspopup="listbox"], [role="banner"] button[aria-haspopup]')]
      .filter(el => visible(el) && /^(chatgpt|gpt)[\s_-]*[0-9]{1,3}(?:[._][0-9]{1,3})?(?=$|[\s_-])/i.test((el.innerText || '').trim()));
    let candidates = exact.length ? exact : fallback;
    let source = 'header';
    // Only the discovered form can supply a compact trigger; ambiguous
    // header controls must never fall through to composer discovery.
    const compactLabel = el => window.__nyx?.compactModelLabel(el.innerText);
    if (!candidates.length) {
      const form = window.__nyx?.discoverControls().input?.closest('form');
      candidates = [...(form?.querySelectorAll('button[aria-haspopup="menu"]') || [])]
        .filter(el => visible(el) && el.getAttribute('data-testid') !== 'composer-plus-btn' && compactLabel(el));
      source = 'composer';
    }
    const trigger = candidates.length === 1 ? candidates[0] : null;
    if (trigger) trigger.setAttribute('data-nyx-switcher', '');
    const rawText = trigger ? (trigger.innerText || trigger.getAttribute('aria-label') || '').trim() : null;
    return { text: trigger && source === 'composer' ? compactLabel(trigger) : rawText,
      rawText, source,
      found: !!trigger, open: (window.__nyx?.modelPickerMenus(pickerId).length || 0) > 0,
      items: (window.__nyx?.modelPickerItems(pickerId) || []).map(el => ({ text: (el.innerText || el.textContent || '').trim() })) };
  }, { deadline: Date.now() + timeout, pickerId: budget.picker?.id }, interactionOptions(budget, 1000)));
  return { ...snapshot, metadata: switcherMetadata(snapshot.text) };
}

export async function selectModelSwitcher(page, requested) {
  await installDomCore(page);
  const budget = interactionBudget(MODEL_SELECT_TIMEOUT_MS, { id: randomUUID() }, { cap: MODEL_SELECT_MAX_MS });
  const stopWatch = watchBudget(budget);
  let result = { verified: false, metadata: 'absent', reason: 'switcher_unverified' };
  try {
    let snapshot = await readModelSwitcher(page, budget);
    result.metadata = snapshot.metadata;
    if (!switcherMatches(snapshot.text, requested) && snapshot.found) {
      const trigger = await page.locator('[data-nyx-switcher]').elementHandle(interactionOptions(budget));
      try {
        await clearRadixLock(page, budget);
        await beginModelPicker(page, budget);
        const current = await readModelSwitcher(page, budget);
        const unchanged = trigger && current.found && current.source === snapshot.source && current.rawText === snapshot.rawText &&
          await boundedRead(budget, timeout => trigger.evaluate((el, { deadline, rawText }) =>
            Date.now() < deadline && el.isConnected && el.hasAttribute('data-nyx-switcher') &&
            (el.innerText || el.getAttribute('aria-label') || '').trim() === rawText,
          { deadline: Date.now() + timeout, rawText: snapshot.rawText }));
        if (!unchanged) throw Object.assign(new Error('picker_changed'), { code: 'picker_changed' });
        await trigger.click(interactionOptions(budget));
      } finally {
        await trigger?.dispose().catch(() => {});
      }
      const menuDeadline = Math.min(budget.deadline, Date.now() + 3000);
      do {
        await budgetPause(budget, 100);
        snapshot = await readModelSwitcher(page, budget);
      } while (!snapshot.open && Date.now() < menuDeadline);
      if (snapshot.open) budget.progress();
      let index = chooseSwitcherEntry(snapshot.items, requested, snapshot.text);
      if (index < 0) {
        const familyIndex = chooseSwitcherFamilyEntry(snapshot.items, requested);
        if (familyIndex >= 0) {
          const family = snapshot.items[familyIndex].text;
          // Resolve the original menu's element before resetting discovery.
          // The click starts a new identity scope, excluding all existing menus.
          await clickPickerElement(page, budget, { index: familyIndex, text: family, beginNested: true });
          const nestedDeadline = Math.min(budget.deadline, Date.now() + 1500);
          do {
            await budgetPause(budget, 100);
            snapshot = await readModelSwitcher(page, budget);
          } while (!snapshot.open && Date.now() < nestedDeadline);
          index = chooseSwitcherEntry(snapshot.items, requested, family);
        }
      }
      if (index >= 0) { await clickPickerElement(page, budget, { index, text: snapshot.items[index].text }); budget.progress(); }
      const verifyDeadline = Math.min(budget.deadline, Date.now() + 1000);
      do {
        await budgetPause(budget, 100);
        snapshot = await readModelSwitcher(page, budget);
      } while (!switcherMatches(snapshot.text, requested) && Date.now() < verifyDeadline);
      if (LOG_PICKER_LABELS && !switcherMatches(snapshot.text, requested)) log(formatPickerLabels({ observed: snapshot.text, items: snapshot.items }));
    }
    result = { verified: switcherMatches(snapshot.text, requested), metadata: snapshot.metadata,
      reason: switcherMatches(snapshot.text, requested) ? 'selected' : 'switcher_unverified' };
  } catch (error) {
    if (stableErrorCode(error) === 'page_crashed') throw error;
  } finally {
    budget.controller.abort();
    stopWatch();
    const cleanup = interactionBudget(2000, budget.picker);
    const cleanupTimer = setTimeout(() => cleanup.controller.abort(), 2000);
    try {
      await boundedRead(cleanup, timeout => page.locator('body').evaluate((_, { id, deadline }) => {
        if (Date.now() >= deadline) return null;
        return window.__nyx?.finishNestedModelPicker(id);
      }, { id: budget.picker.id, deadline: Date.now() + timeout }, interactionOptions(cleanup)));
      for (let i = 0; i < 3 && (await readModelSwitcher(page, cleanup)).open; i += 1) {
        await page.locator('body').press('Escape', interactionOptions(cleanup));
        await budgetPause(cleanup, 100);
      }
    } catch {} finally { cleanup.controller.abort(); clearTimeout(cleanupTimer); }
  }
  return result;
}

export function reportedPromptModel(task) {
  // Keep the request separate from the canonical, read-back observations.
  return task.model;
}

export function modelSelectionDetail(result) {
  const level = MODEL_LEVELS.some((aliases) => aliases[0] === result.level) ? result.level : "custom";
  if (result.reason === "timeout") return "timeout";
  if (result.verified) return `selected=${level}`;
  if (result.reason === "unverified") return `unverified=${level}`;
  return result.reason;
}

// Use the same preference for structural pills and composer-local fallbacks.
// Whether a picker menu offers the Pro Standard / Pro Extended split at all.
// Some accounts no longer have it: the menu lists model versions instead, and
// carries the tier only as an unselectable heading.
export function splitTierOffered(items) {
  return (items || []).some((item) => ["pro_extended", "pro_standard"].includes(effortMetadata(item?.text)));
}

export function preferredModelPillIndex(labels) {
  if (!labels.length) return -1;
  const recognized = labels.findIndex((text) => detectPillLevel(text) !== null);
  if (recognized >= 0) return recognized;
  const legacy = labels.findIndex((text) => /instant|medium|high|extra|pro|gpt|思考|扩展|极速|均衡|高级|超高|\b\d(?:\.\d+)?\b/i.test(text));
  if (legacy >= 0) return legacy;
  // Never fall back to index 0 blindly. The composer region also holds
  // icon-only menu buttons such as composer-plus-btn ("Add files and more"),
  // whose label is empty; picking one clicks the wrong control and no model
  // menu ever opens. Prefer the first candidate that at least has a label.
  return labels.findIndex((text) => String(text || '').trim().length > 0);
}

export function modelSelectionDiagnostics(snapshot) {
  const source = snapshot?.pill ? (snapshot.pill.structural ? "structural" : "fallback") : "none";
  const observed = snapshot?.observed || "";
  const items = snapshot?.items || [];
  const recognized = [...new Set(items.map((item) => detectPillLevel(item.text)).filter(Boolean))];
  return `pill_source=${source} pill_level=${detectPillLevel(observed) || "unrecognized"} ` +
    `pill_text_length=${observed.length} items=${items.length} recognized=[${recognized.join(",")}]`;
}

// Opt-in, local diagnostics only. Call with the composer picker's snapshot,
// never page-wide text; JSON escaping keeps every label on one log line.
export function formatPickerLabels(snapshot) {
  const truncate = (label) => [...String(label ?? "")].slice(0, 40).join("");
  const encode = (value) => JSON.stringify(value).replace(/\u2028/g, "\\u2028").replace(/\u2029/g, "\\u2029");
  const items = (snapshot?.items || []).slice(0, 24).map((item) => truncate(item.text));
  return `picker_labels pill=${encode(truncate(snapshot?.observed))} items=${encode(items)}`;
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

export function interactionBudget(duration, picker = null, { cap = duration, now = Date.now() } = {}) {
  const budget = { deadline: now + duration, controller: new AbortController(), picker,
    window: duration, hardDeadline: now + Math.max(duration, cap) };
  // Progress restarts the window; the hard deadline never moves.
  budget.progress = (at = Date.now()) => {
    if (!budget.controller.signal.aborted) budget.deadline = Math.min(budget.hardDeadline, at + budget.window);
    return budget.deadline;
  };
  return budget;
}

// Abort a budget as soon as its (possibly extended) deadline passes.
function watchBudget(budget, onExpire = () => {}) {
  const timer = setInterval(() => {
    if (Date.now() >= budget.deadline && !budget.controller.signal.aborted) {
      budget.controller.abort();
      onExpire();
    }
  }, 100);
  return () => clearInterval(timer);
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

// Retry only the final read-back, sharing one deadline across both controls
// and all attempts. A late read cannot publish observations or start a retry.
export async function retryPresendModelRead(read) {
  const budget = interactionBudget(PRE_SEND_ACTION_MS);
  let timer;
  const deadline = new Promise((_, reject) => {
    timer = setTimeout(() => {
      budget.controller.abort();
      reject(interactionDeadlineError());
    }, Math.max(0, budget.deadline - Date.now()));
  });
  try {
    for (let attempt = 0; attempt < 3; attempt += 1) {
      try {
        interactionOptions(budget);
        const observed = await Promise.race([read(budget), deadline]);
        interactionOptions(budget);
        return observed;
      } catch (error) {
        if (['page_crashed', 'cdp_disconnected'].includes(stableErrorCode(error))) throw error;
        if (budget.controller.signal.aborted || Date.now() >= budget.deadline) break;
      }
    }
    return null;
  } finally {
    budget.controller.abort();
    clearTimeout(timer);
  }
}

// Synchronous, read-only snapshots avoid N per-item auto-waits. Discovery is
// restricted to the structural pill or the textarea's own composer region.
// Raw labels are logged only with the explicit picker-label diagnostic opt-in,
// and are never written to acknowledgement metadata.
export async function pickerSnapshot(page, budget = interactionBudget(1000)) {
  const snapshot = await boundedRead(budget, (timeout) => page.locator("body").evaluate((body, { composerSelector, sendSelector, deadline, pickerId }) => {
    if (Date.now() >= deadline) return null;
    const visible = (el) => {
      const rect = el.getBoundingClientRect();
      const style = getComputedStyle(el);
      return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none";
    };
    window.__nyx?.discoverControls();
    const input = body.querySelector(composerSelector);
    const form = input?.closest("form");
    let region = form || input?.parentElement;
    if (!form) while (region && !region.querySelector(sendSelector)) region = region.parentElement;
    if (region === body || region === document.documentElement) region = null;
    // Model tiers are not effort evidence, even when the trigger looks like a pill.
    let pills = [...body.querySelectorAll('button.__composer-pill[aria-haspopup="menu"]:not([data-nyx-switcher])')].filter(visible);
    // Self-heal a stranded switcher marker. readModelSwitcher stamps
    // data-nyx-switcher on whichever control it claims, and on a page with no
    // header switcher that claim lands on the composer pill itself - so the
    // selector above skips the only pill there is and we report
    // pill_source=none. Nothing else clears the attribute: readModelSwitcher
    // is the sole clear site and it re-stamps the same pill on the next
    // attempt, so the worker rebuilds the fault on every retry and can never
    // select a model again. Unmark a marked composer pill only when it left us
    // with no candidate at all; a marker on a real header switcher is
    // load-bearing (it is how the effort step avoids re-picking the switcher)
    // and must stay.
    if (!pills.length) {
      const stranded = [...body.querySelectorAll('button.__composer-pill[aria-haspopup="menu"][data-nyx-switcher]')].filter(visible);
      if (stranded.length) {
        stranded.forEach((el) => el.removeAttribute('data-nyx-switcher'));
        pills = stranded;
      }
    }
    const candidates = pills.length ? pills : [...(region?.querySelectorAll('button[aria-haspopup="menu"]:not([data-nyx-switcher])') || [])].filter(visible);
    const menus = window.__nyx?.modelPickerMenus(pickerId) || [];
    const items = (window.__nyx?.modelPickerItems(pickerId) || []).map((el) => ({
      text: (el.innerText || el.textContent || "").trim(),
      checked: el.getAttribute("aria-checked") === "true" || el.getAttribute("aria-selected") === "true",
    }));
    return {
      // Adapt a compact pill label the way readModelSwitcher and
      // chooseSwitcherEntry already do. The composer pill renders family and
      // tier on separate lines ("6\nPro"); every metadata helper deliberately
      // refuses to read an un-adapted compact label, so leaving it raw makes
      // pillShowsLevel false and effortMetadata 'unrecognized' for a pill that
      // plainly shows Pro - the worker then skips already_selected, hunts a
      // Pro Extended entry the menu lacks, and fails level_unavailable.
      candidates: candidates.map((el) => {
        const raw = (el.innerText || el.textContent || "").trim();
        return window.__nyx?.compactModelLabel(raw) || raw;
      }),
      structural: !!pills.length, form: !!form,
      open: menus.length > 0, items, submenu: !!window.__nyx?.modelPickerTrigger(pickerId),
    };
  }, { composerSelector: COMPOSER_SELECTOR, sendSelector: SEND_SELECTOR, deadline: Date.now() + timeout,
    pickerId: budget.picker?.id }, interactionOptions(budget, 1000)));
  interactionOptions(budget);
  const index = preferredModelPillIndex(snapshot.candidates);
  snapshot.pill = index < 0 ? null : { index, structural: snapshot.structural, form: snapshot.form };
  snapshot.observed = snapshot.candidates[index] || null;
  if (budget.picker) {
    // Preserve the last open picker's items after Escape for diagnostics.
    // Default logging projects canonical metadata; raw labels require opt-in.
    budget.picker.recognizedObservation ||= snapshot.candidates.some(text => detectPillLevel(text) !== null);
    budget.picker.recognizedLevels ||= snapshot.items.some(item => detectPillLevel(item.text) !== null);
    const lastItems = budget.picker.snapshot?.items || [];
    budget.picker.snapshot = { ...snapshot, items: snapshot.open ? snapshot.items : lastItems };
  }
  return snapshot;
}

// Clear a leftover Radix modal lock before recording pre-existing menus.
// Persistent sidebar menus alone never trigger Escape here.
async function clearRadixLock(page, budget) {
  for (let escapes = 0; escapes < 3; escapes += 1) {
    const locked = await boundedRead(budget, (timeout) => page.locator("body").evaluate((body, deadline) => {
      if (Date.now() >= deadline) return null;
      return getComputedStyle(body).pointerEvents === "none";
    }, Date.now() + timeout, interactionOptions(budget, 1000)));
    if (!locked) return;
    await page.locator("body").press("Escape", interactionOptions(budget));
    await budgetPause(budget, 100);
  }
}

async function beginModelPicker(page, budget, nested = false) {
  await boundedRead(budget, (timeout) => page.locator("body").evaluate((_, { id, deadline, nested }) => {
    if (Date.now() >= deadline) return null;
    return window.__nyx?.beginModelPicker(id, nested);
  }, { id: budget.picker.id, deadline: Date.now() + timeout, nested }, interactionOptions(budget, 1000)));
}

function pickerLocator(page, pill) {
  if (pill.structural) return page.locator(PILL_SELECTOR).nth(pill.index);
  const region = page.locator(COMPOSER_SELECTOR).first().locator(pill.form ? "xpath=ancestor::form[1]" : COMPOSER_REGION_XPATH);
  return region.locator('button[aria-haspopup="menu"]:visible:not([data-nyx-switcher])').nth(pill.index);
}

async function clickPickerElement(page, budget, entry) {
  let handle;
  let acceptingHandle = true;
  try {
    handle = await boundedRead(budget, (timeout) => page.locator("body").evaluateHandle((_, { id, deadline, entry }) => {
      if (Date.now() >= deadline) return "deadline";
      return entry.trigger ? window.__nyx?.modelPickerTrigger(id)
        : window.__nyx?.modelPickerItem(id, entry.index, entry.text);
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
    if (!element) {
      // Wrap the value: a genuine missing item returns null, which is distinct
      // from the page-side deadline marker and valid for this lookup.
      const { value } = await boundedRead(budget, async () => ({ value: await handle.jsonValue() }));
      if (value === "deadline") throw interactionDeadlineError();
      throw Object.assign(new Error("picker_changed"), { code: "picker_changed" });
    }
    if (entry.beginNested) await beginModelPicker(page, budget, true);
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
  budget.picker.expectedEffort = effortMetadata(snapshot.items[index].text);
  await clickPickerElement(page, budget, { index, text: snapshot.items[index].text });
  budget.progress();
  return true;
}

async function closeOpenMenus(page, budget) {
  for (let escapes = 0; escapes < 3 && (await pickerSnapshot(page, budget)).open; escapes += 1) {
    await page.locator("body").press("Escape", interactionOptions(budget));
    await budgetPause(budget, 100);
  }
}

// ── Reasoning-effort slider ──────────────────────────────────────────────
// The current composer picker exposes the reasoning level as a Radix slider
// (role="slider", aria-valuemin/max/now) under a "Power" menu item, with the
// model family/version in a separate "Select model" submenu. Driving the
// slider by its ARIA state is what codex-chatgpt-web (MIT) does; it survives
// label, layout and locale changes that broke every text-matching path here.
// Levels are ordered Instant, Medium, High, Extra High, Pro from the minimum.
// A range shorter than five entries hides the top levels (Pro disappears when
// its usage limit is reached), never the bottom ones.
const EFFORT_SLIDER_CONTAINER_SELECTOR = '[data-model-reasoning-effort-slider]';
const EFFORT_SLIDER_SELECTOR = '[data-model-reasoning-effort-slider] [role="slider"]';
const EFFORT_SLIDER_LEVELS = ["Instant", "Medium", "High", "Extra High", "Pro"];
const EFFORT_SLIDER_STEP_MS = 1000;

export function effortSliderIndex(level) {
  return EFFORT_SLIDER_LEVELS.indexOf(level);
}

function safeIntegerAttribute(value) {
  if (value === null || value === undefined || !/^-?\d+$/.test(String(value))) return undefined;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : undefined;
}

export function parseEffortSliderState(rawMin, rawMax, rawValue) {
  const min = safeIntegerAttribute(rawMin);
  const max = safeIntegerAttribute(rawMax);
  const value = safeIntegerAttribute(rawValue);
  if (min === undefined || max === undefined || value === undefined) return null;
  const options = max - min + 1;
  if (options < 1 || options > EFFORT_SLIDER_LEVELS.length) return null;
  if (value < min || value > max) return null;
  return { min, max, value };
}

// How to move a parsed slider onto a canonical level: the signed number of
// single-step key presses, or unavailable when the range does not reach it.
export function effortSliderPlan(state, level) {
  const index = effortSliderIndex(level);
  if (!state || index < 0) return { steps: null, unavailable: true, hint: "unsupported" };
  const target = state.min + index;
  if (target > state.max) {
    return { steps: null, unavailable: true, target,
      hint: level === "Pro" && state.max - state.min === 3 ? "pro_hidden_usage_limit" : "range_too_short" };
  }
  return { steps: target - state.value, unavailable: false, target };
}

function effortSliderLocators(page) {
  const container = page.locator(EFFORT_SLIDER_CONTAINER_SELECTOR).filter({ visible: true }).last();
  const slider = container.locator('[role="slider"]');
  return { container, slider, control: slider.locator("xpath=ancestor::*[@role='menuitem'][1]") };
}

// Read the slider inside this picker's own menus only; null when absent.
async function effortSliderState(page, budget) {
  return boundedRead(budget, (timeout) => page.locator("body").evaluate((body, { deadline, pickerId, selector }) => {
    if (Date.now() >= deadline) return null;
    const menus = window.__nyx?.modelPickerMenus(pickerId) || [];
    const slider = [...body.querySelectorAll(selector)].find((el) => {
      const container = el.closest("[data-model-reasoning-effort-slider]");
      const rect = container?.getBoundingClientRect();
      return rect && rect.width > 0 && rect.height > 0 && menus.includes(el.closest('[role="menu"], [role="listbox"]'));
    });
    if (!slider) return { absent: true };
    return { absent: false, min: slider.getAttribute("aria-valuemin"), max: slider.getAttribute("aria-valuemax"),
      value: slider.getAttribute("aria-valuenow") };
  }, { deadline: Date.now() + timeout, pickerId: budget.picker?.id, selector: EFFORT_SLIDER_SELECTOR },
  interactionOptions(budget, 1000))).then((read) => (read.absent ? null : parseEffortSliderState(read.min, read.max, read.value)));
}

async function waitForEffortSliderValue(page, budget, previous, until) {
  let state;
  do {
    state = await effortSliderState(page, budget);
    if (state && state.value !== previous) return state;
    await budgetPause(budget, 50);
  } while (Date.now() < until);
  return state;
}

// Returns false when the open menu has no slider (legacy picker: caller falls
// back to text matching). Otherwise sets result.reason and returns true.
async function selectEffortBySlider(page, targets, budget, result, state) {
  const plan = effortSliderPlan(state, targets[0]);
  if (plan.unavailable && plan.hint === "unsupported") return false;
  budget.picker.recognizedLevels = true;
  // The menu is open here, so this snapshot carries the version radios.
  budget.picker.family = familyFromModelRadios((await pickerSnapshot(page, budget)).items);
  budget.picker.slider = { min: state.min, max: state.max, before: state.value, hint: plan.hint || null };
  if (plan.unavailable) {
    await closeOpenMenus(page, budget);
    result.reason = "level_unavailable";
    return true;
  }
  if (plan.steps === 0) {
    await closeOpenMenus(page, budget);
    result.reason = "already_selected";
    return true;
  }
  const { control } = effortSliderLocators(page);
  const key = plan.steps > 0 ? "ArrowRight" : "ArrowLeft";
  const direction = plan.steps > 0 ? 1 : -1;
  let current = state;
  while (current.value !== plan.target) {
    const previous = current.value;
    await control.press(key, interactionOptions(budget));
    current = await waitForEffortSliderValue(page, budget, previous, Math.min(budget.deadline, Date.now() + EFFORT_SLIDER_STEP_MS));
    if (current && current.value !== previous) budget.progress();
    if (!current || current.value !== previous + direction) {
      // The slider moved unexpectedly or stalled: stop and let the pill decide.
      await closeOpenMenus(page, budget);
      result.reason = "unverified";
      return true;
    }
  }
  budget.picker.slider.after = current.value;
  await closeOpenMenus(page, budget);
  // Reopen once: ChatGPT can drop a keyboard selection when the menu closes.
  await budgetPause(budget, 200);
  const before = await pickerSnapshot(page, budget);
  if (before.pill) {
    await pickerLocator(page, before.pill).click(interactionOptions(budget));
    const reopened = await waitForEffortSliderValue(page, budget, null, Math.min(budget.deadline, Date.now() + 3000));
    if (reopened) budget.progress();
    budget.picker.slider.confirmed = reopened?.value ?? null;
    await closeOpenMenus(page, budget);
    if (!reopened || reopened.value !== plan.target) {
      result.reason = "unverified";
      return true;
    }
  }
  result.reason = "unverified"; // promoted to "selected" once the pill agrees
  return true;
}

export function effortSliderDetail(slider) {
  if (!slider) return "slider=absent";
  const range = `${slider.min}-${slider.max}`;
  const path = [slider.before, slider.after, slider.confirmed].filter((v) => v !== undefined && v !== null).join(">");
  return `slider=${path}/${range}${slider.hint ? ` hint=${slider.hint}` : ""}`;
}

// Returns { level, verified, observed, reason }. Only the actual pill can
// verify a level or populate observed. Selection never throws into the task
// flow. Abort cancels Playwright actions, and the deadline is checked before
// EVERY interaction, including after reads that resolve late. The backstop
// drains the inner promise before menu cleanup; no detached selection loop
// can race prompt typing or Send.
export async function selectModel(page, modelLabel) {
  await installDomCore(page);
  const targets = modelLevelTargets(modelLabel);
  const result = { level: targets[0] || null, verified: false, observed: null, reason: "picker_unavailable" };
  const budget = interactionBudget(MODEL_SELECT_TIMEOUT_MS, { id: randomUUID(), snapshot: null }, { cap: MODEL_SELECT_MAX_MS });
  let drainTimer;
  const inner = selectModelInner(page, targets, budget, result).catch((error) => {
    if (stableErrorCode(error) === "page_crashed") result.failureCode = "page_crashed";
    result.reason = modelSelectionFailureReason(error, {
      deadline: budget.deadline, aborted: budget.controller.signal.aborted,
    }, Date.now());
  });
  // The watchdog aborts on silence (cancelling even a click waiting for
  // actionability); progress inside selectModelInner keeps extending it.
  let stopWatch;
  const timeout = new Promise((resolveTimeout) => { stopWatch = watchBudget(budget, () => resolveTimeout("timeout")); });
  try {
    if (await Promise.race([inner, timeout]) === "timeout") {
      await Promise.race([inner, new Promise((resolveDrain) => { drainTimer = setTimeout(resolveDrain, 3000); })]);
      result.reason = "timeout";
    }
  } finally {
    budget.controller.abort();
    stopWatch();
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
  if (result.failureCode) throw Object.assign(new Error(result.failureCode), { code: result.failureCode });
  result.verified = pillShowsLevel(result.observed, targets) &&
    !["timeout", "menu_not_opened", "interaction_deadline", "selection_failed", "level_unavailable"].includes(result.reason) &&
    (!["pro_extended", "pro_standard"].includes(budget.picker.expectedEffort) || effortMetadata(result.observed) === budget.picker.expectedEffort);
  if (result.verified && !["timeout", "already_selected"].includes(result.reason)) result.reason = "selected";
  log(`model_selection reason=${result.reason} ${modelSelectionDetail(result)} ${modelSelectionDiagnostics(budget.picker.snapshot)} ${effortSliderDetail(budget.picker.slider)} family=${budget.picker.family || 'absent'}`);
  if (LOG_PICKER_LABELS && ["level_unavailable", "unverified", "menu_not_opened", "picker_unavailable"].includes(result.reason)) {
    log(formatPickerLabels(budget.picker.snapshot));
  }
  return { ...result, recognizedLevels: !!budget.picker.recognizedLevels, recognizedObservation: !!budget.picker.recognizedObservation,
    family: budget.picker.family || 'absent' };
}

async function selectModelInner(page, targets, budget, result) {
  const before = await pickerSnapshot(page, budget);
  interactionOptions(budget);
  result.observed = before.observed;
  if (pillShowsLevel(before.observed, targets) && (targets[0] !== "Pro" || effortMetadata(before.observed) !== "pro")) {
    result.reason = "already_selected";
    return;
  }
  if (!before.pill || !targets.length) return;
  await clearRadixLock(page, budget);
  await beginModelPicker(page, budget);
  await pickerLocator(page, before.pill).click(interactionOptions(budget));
  const menuWait = interactionOptions(budget, 5000);
  try {
    await page.locator("body").waitForFunction((_, id) => window.__nyx?.modelPickerMenus(id).length > 0,
      budget.picker.id, menuWait);
    budget.progress();
  } catch (error) {
    interactionOptions(budget);
    if (error?.name !== "TimeoutError") throw error;
    result.reason = menuWait.timeout < 5000 ? "timeout" : "menu_not_opened";
    return;
  }
  // Prefer the ARIA slider; fall back to text matching on an older picker.
  await budgetPause(budget, 100);
  const slider = await effortSliderState(page, budget);
  if (slider && (await selectEffortBySlider(page, targets, budget, result, slider))) {
    if (result.reason !== "unverified") return;
    const verifyUntil = Math.min(budget.deadline, Date.now() + 1000);
    while (true) {
      const after = await pickerSnapshot(page, budget);
      interactionOptions(budget);
      result.observed = after.observed;
      if (pillShowsLevel(result.observed, targets) || Date.now() >= verifyUntil) return;
      await budgetPause(budget, 100);
    }
  }
  let clicked = await clickMatchingLevel(page, targets, budget);
  if (!clicked && (await pickerSnapshot(page, budget)).submenu) {
    await clickPickerElement(page, budget, { trigger: true });
    await budgetPause(budget, 200);
    clicked = await clickMatchingLevel(page, targets, budget);
  }
  if (!clicked) {
    interactionOptions(budget);
    // ChatGPT has removed the Pro Standard / Pro Extended split on some
    // accounts. The pill's menu now lists model versions - Latest, GPT-5.6,
    // GPT-5.5 - and carries "6 Pro" only as a heading with aria-checked unset,
    // so it cannot be clicked. modelLevelTargets still asks for Pro Extended on
    // every Pro request (no pool setting yields a bare Pro), so the hunt can
    // never succeed and every task dies level_unavailable while the pill sits
    // on the requested model the whole time. Observed 2026-09-21: a 15-worker
    // pool fully down, each task rerouted through all 15 before failing.
    // When nothing in the menu offers the split, the pill already shows the
    // requested level and there is nothing left to click.
    if (targets[0] === "Pro" && pillShowsLevel(result.observed, targets) &&
        !splitTierOffered(budget.picker?.snapshot?.items)) {
      await closeOpenMenus(page, budget);
      result.reason = "already_selected";
      return;
    }
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

// Dismiss ChatGPT's Temporary Chat onboarding modal (a native <dialog> that
// intercepts every pointer event until Continue is pressed). Best-effort: if
// it stays, ensureComposerUnobstructed fails the task pre-send as before.
export async function dismissTemporaryChatOnboarding(page) {
  const modal = page.locator(TEMPORARY_CHAT_ONBOARDING_SELECTOR).last();
  try {
    if (!(await modal.isVisible().catch(() => false))) return false;
    const actions = [
      modal.getByRole("button", { name: "Continue", exact: true }).last(),
      modal.locator('button:not([data-testid="close-button"])').last(),
      modal.locator('button[data-testid="close-button"]').last(),
    ];
    for (const action of actions) {
      if (!(await action.isVisible().catch(() => false))) continue;
      await action.click({ force: true, timeout: PRE_SEND_ACTION_MS });
      await modal.waitFor({ state: "hidden", timeout: PRE_SEND_ACTION_MS });
      log("temporary_chat onboarding dismissed");
      return true;
    }
  } catch (error) {
    if (stableErrorCode(error) === "page_crashed") throw error;
  }
  return false;
}

// Clear overlays before typing and again immediately before Send. Never force
// a click through an obstruction: failure stays pre-send and enters the
// existing browser recovery / infrastructure retry path.
// Clear a stale draft left in the composer by an earlier attempt on this tab.
// Model selection runs before the prompt is typed, but a very long leftover
// draft makes the page heavy enough that the selection interactions exceed
// MODEL_SELECT_TIMEOUT_MS and the task fails as operation_timeout@selecting_model
// on every subsequent pickup. The prompt is (re)typed after selection, so
// clearing here is a no-op on a fresh composer and never drops real work.
async function clearComposerDraft(page) {
  const budget = interactionBudget(PRE_SEND_ACTION_MS);
  const timer = setTimeout(() => budget.controller.abort(), PRE_SEND_ACTION_MS);
  try {
    const draft = await boundedRead(budget, (timeout) => page.locator("body").evaluate((body, { composerSelector, deadline }) => {
      if (Date.now() >= deadline) return { head: "", length: 0 };
      window.__nyx?.discoverControls();
      const input = body.querySelector(composerSelector);
      if (!input) return { head: "", length: 0 };
      const text = String(input.value ?? input.innerText ?? "");
      return { head: text.trim().slice(0, 8), length: text.length };
    }, { composerSelector: COMPOSER_SELECTOR, deadline: Date.now() + timeout }, interactionOptions(budget, 1000)));
    if (!composerHasDraft(draft.head)) return;
    // Drop an oversized draft in a single DOM assignment rather than typing it
    // away; the prompt is (re)typed after selection, so nothing real is lost.
    if (draftNeedsFastClear(draft.length)) {
      const emptied = await boundedRead(budget, (timeout) => page.locator("body").evaluate((body, { composerSelector, deadline }) => {
        if (Date.now() >= deadline) return false;
        const input = body.querySelector(composerSelector);
        if (!input) return false;
        if (typeof input.value === "string") input.value = "";
        else { input.focus(); input.textContent = ""; }
        input.dispatchEvent(new InputEvent("input", { bubbles: true }));
        return String(input.value ?? input.innerText ?? "").trim().length === 0;
      }, { composerSelector: COMPOSER_SELECTOR, deadline: Date.now() + timeout }, interactionOptions(budget, 1000)));
      if (emptied) return;
    }
    const input = page.locator(COMPOSER_SELECTOR).first();
    await input.click(interactionOptions(budget)).catch(() => {});
    await input.fill("", interactionOptions(budget)).catch(() => {});
  } catch (error) {
    if (stableErrorCode(error) === "page_crashed") throw error;
    // Best-effort: a failed clear must not consume a recovery attempt.
  } finally {
    budget.controller.abort();
    clearTimeout(timer);
  }
}

async function ensureComposerUnobstructed(page) {
  const budget = interactionBudget(PRE_SEND_ACTION_MS);
  const timer = setTimeout(() => budget.controller.abort(), PRE_SEND_ACTION_MS);
  try {
    await page.locator(COMPOSER_SELECTOR).first().scrollIntoViewIfNeeded(interactionOptions(budget)).catch(() => {});
    while (true) {
      const state = await boundedRead(budget, (timeout) => page.locator("body").evaluate((body, { composerSelector, deadline }) => {
        if (Date.now() >= deadline) return null;
        window.__nyx?.discoverControls();
        const input = body.querySelector(composerSelector);
        const rect = input?.getBoundingClientRect();
        // Hit-test the composer's VISIBLE centre, not its geometric centre. A
        // long draft can make the composer taller than the viewport and push
        // its midpoint far off screen, where elementFromPoint returns null and
        // the composer is wrongly judged obstructed. Intersect the composer rect
        // with the viewport and every scroll/clip ancestor, then test the middle
        // of what remains. Keep this in sync with composerVisibleHitPoint (same
        // maths, separate runtime: this copy runs in the page and cannot import).
        let visible = rect && {
          left: Math.max(0, rect.left),
          right: Math.min(window.innerWidth, rect.right),
          top: Math.max(0, rect.top),
          bottom: Math.min(window.innerHeight, rect.bottom),
        };
        for (let ancestor = input?.parentElement; visible && ancestor; ancestor = ancestor.parentElement) {
          const style = getComputedStyle(ancestor);
          const bounds = ancestor.getBoundingClientRect();
          if (/auto|scroll|hidden|clip/.test(style.overflowX)) {
            visible.left = Math.max(visible.left, bounds.left);
            visible.right = Math.min(visible.right, bounds.right);
          }
          if (/auto|scroll|hidden|clip/.test(style.overflowY)) {
            visible.top = Math.max(visible.top, bounds.top);
            visible.bottom = Math.min(visible.bottom, bounds.bottom);
          }
        }
        const hasVisibleArea = !!visible && visible.right > visible.left && visible.bottom > visible.top;
        const hit = hasVisibleArea && document.elementFromPoint((visible.left + visible.right) / 2, (visible.top + visible.bottom) / 2);
        const main = body.querySelector("main");
        const mainRect = main?.getBoundingClientRect();
        const neutral = mainRect && document.elementFromPoint(mainRect.x + 4, mainRect.y + 4) === main;
        return { clear: !!hit && input.contains(hit) && getComputedStyle(body).pointerEvents !== "none", neutral };
      }, { composerSelector: COMPOSER_SELECTOR, deadline: Date.now() + timeout }, interactionOptions(budget, 1000)));
      interactionOptions(budget);
      if (state.clear) return;
      await page.locator("body").press("Escape", interactionOptions(budget));
      if (!state.clear && state.neutral) {
        await page.locator("main").first().click({ position: { x: 4, y: 4 }, ...interactionOptions(budget) }).catch(() => {});
      }
      await budgetPause(budget, 100);
    }
  } catch (error) {
    if (stableErrorCode(error) === "page_crashed") throw error;
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
  await installDomCore(page);
  return page.evaluate(() => {
    const main = document.querySelector("main");
    const composer = window.__nyx?.discoverControls().input;
    return {
      ready: Boolean(main && composer && window.__nyx),
      errorCode: window.__nyx?.errorCode(),
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
      // A Temporary Chat has no conversation URL worth storing; the bare
      // ?temporary-chat=true address would only open an empty chat.
      chatgpt_url: promptUsesTemporaryChat(task) && !convId(page.url()) ? null : page.url(),
      // Observations are canonical metadata, never raw picker labels.
      model: reportedPromptModel(task),
      observed_model_switcher: runtime.state.current_task?.observed_model_switcher,
      observed_model_effort: runtime.state.current_task?.observed_model_effort,
    })
  );
  log(
    `prompt ${task.task_id} -> ${result.status} (${response.length} chars, ` +
      `${downloadedImages.items.length} image(s)/${downloadedImages.bytes}B, ` +
      `${downloadedFiles.items.length} file(s)/${downloadedFiles.bytes}B)`
  );
}

async function failModelSelection(runtime, task, switcher, effort, reason) {
  updateTaskState(runtime.state, { observed_model_switcher: switcher, observed_model_effort: effort });
  if (await ack(runtime, task, 'selecting_model', `switcher=${switcher} effort=${effort} reason=${reason}`)) {
    throw new TaskFailure('cancelled');
  }
  throw new TaskFailure('model_unavailable');
}

async function handlePrompt(runtime, page, task, recovering) {
  const { task_id } = task;
  task.model ||= "chatgpt-6-pro";
  if (task.require_model_match === true && task.model === "unknown") throw new TaskFailure("model_unavailable");
  // Fail closed before the composer is touched: an undeliverable prompt must
  // not spend the fill allowance or leave a draft on this tab.
  if (promptExceedsLimit(task.prompt?.length)) {
    log(`prompt ${task_id} refused: ${task.prompt.length} chars exceeds NYXID_MAX_PROMPT_CHARS=${PROMPT_MAX_CHARS}`);
    throw new TaskFailure("prompt_too_long");
  }
  log(`prompt task ${task_id} (followup=${!!task.is_followup})`);
  await page.bringToFront().catch(() => {});

  // Navigate: continue an existing conversation, or start a FRESH chat.
  // For a fresh prompt we must leave any /c/<uuid> page we're parked on,
  // otherwise we'd type into the previous conversation.
  const persistedUrl = runtime.state.current_task?.conversation_url;
  const priorPhase = runtime.state.current_task?.phase || "claimed";
  const temporaryChat = promptUsesTemporaryChat(task);
  const navigation = choosePromptNavigation({
    recovering,
    phase: priorPhase,
    isFollowup: task.is_followup,
    currentUrl: page.url(),
    persistedUrl,
    taskConversationUrl: task.conversation_url,
    requiredProjectUrl: task.required_project_url,
    temporaryChat,
  });
  if (navigation.error) throw new TaskFailure(navigation.error);
  const navTarget = navigation.target;
  if (navTarget) {
    await page.goto(navTarget, { waitUntil: "domcontentloaded" });
    await installDomCore(page);
    await page.bringToFront().catch(() => {});
    // Hydration first, then a quiet DOM: replaces a flat 2.5s sleep that was
    // both too short on a slow page and wasted on a fast one.
    await waitForComposer(page, 15000);
    await settleDom(page, { quietMs: 250, maxMs: 2500 });
    if (temporaryChat) await dismissTemporaryChatOnboarding(page);
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
    await settleDom(page, { quietMs: 250, maxMs: 1500 });
    const snapshot = await transcriptSnapshot(page);
    if (snapshot.errorCode) {
      const answer = await recoverContentFailure(runtime, page, task, snapshot.assistantCount, snapshot.errorCode);
      await submitPromptResult(runtime, page, task, answer.text, answer.images, answer.files);
      return;
    }
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

  await installDomCore(page);
  let readyError = await page.evaluate(() => window.__nyx?.errorCode());
  if (readyError === 'chatgpt_error_response' && !runtime.state.current_task?.pre_send_reload_attempted) {
    updateTaskState(runtime.state, { pre_send_reload_attempted: true });
    await page.reload({ waitUntil: 'domcontentloaded', timeout: 60000 });
    await installDomCore(page);
    await waitForComposer(page, 15000);
    await settleDom(page, { quietMs: 250, maxMs: Math.max(1500, STABLE_INTERVAL_MS) });
    readyError = await page.evaluate(() => window.__nyx?.errorCode());
  }
  if (readyError === 'chatgpt_error_response') {
    // No send has happened: this is a page-shape failure, not task content.
    throw Object.assign(new Error(readyError), { code: readyError });
  }
  if (readyError === 'model_unavailable') {
    const header = await readModelSwitcher(page);
    const pill = await pickerSnapshot(page, interactionBudget(PRE_SEND_ACTION_MS));
    await failModelSelection(runtime, task, header.metadata, effortMetadata(pill.observed), 'model_unavailable');
  }
  if (readyError) throw new TaskFailure(readyError);
  // The Temporary Chat onboarding modal can appear a few seconds after the
  // composer renders; it intercepts every click until Continue is pressed.
  if (temporaryChat) await dismissTemporaryChatOnboarding(page);
  // A stale draft from a prior attempt makes model selection time out; clear
  // it so each attempt selects the model against a light, empty composer.
  await clearComposerDraft(page);
  if (task.model && task.model !== "unknown") {
    if (await ack(runtime, task, "selecting_model")) throw new TaskFailure("cancelled");
    const headerSelection = await selectModelSwitcher(page, task.model);
    // The current composer shows the family only on the Pro pill ("6 Pro");
    // parked on High or Instant it reads just the level, so no switcher can
    // be found before the effort is selected. Defer the family check to the
    // post-selection read-back instead of failing every task on such a tab.
    if (task.require_model_match !== false && !headerSelection.verified && headerSelection.metadata !== "absent") {
      const pill = await pickerSnapshot(page, interactionBudget(PRE_SEND_ACTION_MS));
      await failModelSelection(runtime, task, headerSelection.metadata, effortMetadata(pill.observed), headerSelection.reason);
    }
    const selected = await selectModel(page, task.model);
    const observedSwitcher = await readModelSwitcher(page).catch(error => {
      if (stableErrorCode(error) === "page_crashed") throw error;
      return { text: null, metadata: "absent" };
    });
    // Below Pro the pill shows only the level; the checked version radio read
    // while the picker was open is then the family evidence.
    const familyVerified = switcherMatches(observedSwitcher.text, task.model) ||
      (observedSwitcher.metadata === "absent" && switcherMetadataMatches(selected.family, task.model));
    const familyMetadata = observedSwitcher.metadata === "absent" && familyVerified ? selected.family : observedSwitcher.metadata;
    updateTaskState(runtime.state, { observed_model_switcher: familyMetadata,
      observed_model_effort: effortMetadata(selected.observed), effort_levels_exposed: selected.recognizedLevels });
    // Re-read BOTH controls after selecting effort, since either may change the other.
    if (task.require_model_match !== false && (!familyVerified || effortSelectionMismatch(selected, task.model))) {
      await failModelSelection(runtime, task, familyMetadata, effortMetadata(selected.observed),
        !familyVerified ? 'switcher_unverified' : selected.reason);
    }
    if (await ack(runtime, task, "selecting_model", modelSelectionDetail(selected))) {
      throw new TaskFailure("cancelled");
    }
  }

  // Type the prompt into the composer (native — more robust than the
  // userscript's execCommand fallbacks) and send.
  const input = page.locator(COMPOSER_SELECTOR).first();
  await page.evaluate(() => window.__nyx?.discoverControls());
  try {
    const deadline = Date.now() + 15000;
    while (!(await page.evaluate(() => !!window.__nyx?.discoverControls().input))) {
      if (Date.now() >= deadline) throw new Error('composer_not_found');
      await sleep(100);
    }
    await input.waitFor({ state: "visible", timeout: PRE_SEND_ACTION_MS });
  } catch (error) {
    if (stableErrorCode(error) === 'page_crashed') throw error;
    throw Object.assign(new Error('composer_not_found'), { code: 'composer_not_found' });
  }
  await ensureComposerUnobstructed(page);
  await input.click({ timeout: PRE_SEND_ACTION_MS });
  await input.fill(task.prompt, { timeout: promptFillTimeout(task.prompt?.length) });
  const typed = await input.evaluate(el => el.value ?? el.innerText);
  if (normalizePromptText(typed) !== normalizePromptText(task.prompt)) throw Object.assign(new Error('composer_readback_failed'), { code: 'composer_readback_failed' });
  await installDomCore(page);
  const before = await boundedRead(interactionBudget(PRE_SEND_ACTION_MS), (timeout) =>
    page.locator("body").evaluate(() => ({
      baseline: window.__nyx?.extractTranscript()?.length || 0,
      assistantCount: window.__nyx?.assistantCount() || 0,
    }), undefined, { timeout }));
  const baseline = before.baseline;
  updateTaskState(runtime.state, { phase: "ready_to_send", baseline_turn_count: baseline });
  if (await ack(runtime, task, "ready_to_send")) throw new TaskFailure("cancelled");
  await settleDom(page, { quietMs: 100, maxMs: 300 });
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
  try { await sendBtn.click({ trial: true, timeout: PRE_SEND_ACTION_MS }); }
  catch (error) {
    if (stableErrorCode(error) === 'page_crashed') throw error;
    throw Object.assign(new Error('send_button_not_found'), { code: 'send_button_not_found' });
  }
  if (!task.is_followup && (task.pdf_base64 || task.attachment_base64)) {
    if (await ack(runtime, task, "ready_to_send")) throw new TaskFailure("cancelled");
  }
  if (task.model !== 'unknown') {
    await installDomCore(page);
    const { header, pill } = await retryPresendModelRead(async budget => {
      const header = await readModelSwitcher(page, budget);
      const pill = await pickerSnapshot(page, budget);
      return { header, pill };
    }) || { header: { text: null, metadata: 'absent' }, pill: { observed: null } };
    const observedEffort = effortMetadata(pill.observed);
    const previousEffort = runtime.state.current_task?.observed_model_effort;
    const recognizedBefore = previousEffort && !['absent', 'unrecognized'].includes(previousEffort);
    const verifiedEffort = pillShowsLevel(pill.observed, modelLevelTargets(task.model)) &&
      (!['pro_extended', 'pro_standard'].includes(previousEffort) || previousEffort === observedEffort);
    // The family cannot change without the picker; when the pill hides it
    // (every level below Pro) the family verified at selection still stands,
    // provided the pill still shows the requested level.
    const previousFamily = runtime.state.current_task?.observed_model_switcher;
    const familyVerified = switcherMatches(header.text, task.model) ||
      (header.metadata === 'absent' && verifiedEffort && switcherMetadataMatches(previousFamily, task.model));
    const familyMetadata = header.metadata === 'absent' && familyVerified ? previousFamily : header.metadata;
    updateTaskState(runtime.state, { observed_model_switcher: familyMetadata, observed_model_effort: observedEffort });
    if (task.require_model_match !== false && (!familyVerified ||
      effortSelectionMismatch({ observed: pill.observed, verified: verifiedEffort,
        recognizedLevels: runtime.state.current_task?.effort_levels_exposed || recognizedBefore }, task.model))) {
      await failModelSelection(runtime, task, familyMetadata, observedEffort, 'presend_unverified');
    }
  }
  // Watch this page's own conversation POST so a server-side rejection of the
  // message (HTTP 413) is named as such instead of surfacing as a stalled turn.
  const rejection = observeSubmissionRejection(page);
  let answer;
  try {
    updateTaskState(runtime.state, { phase: "send_attempted", baseline_turn_count: baseline });
    await sendBtn.click({ timeout: PRE_SEND_ACTION_MS });
    updateTaskState(runtime.state, { phase: "sent" });
    await ack(runtime, task, "sent");
    await pinCurrentConversation(runtime, page, task);
    answer = await waitForResponse(runtime, page, task, beforeCount, rejection);
  } finally {
    rejection.stop();
  }
  await submitPromptResult(runtime, page, task, answer.text, answer.images, answer.files);
}

function convId(url) {
  const m = (url || "").match(/\/c\/([a-f0-9-]{6,})/);
  return m ? m[1] : null;
}

// Returns the latest assistant turn's text plus on-page image and file sources.
// Artifact-only turns are valid; the stability key spans all three outputs.
async function waitForResponse(runtime, page, task, beforeCount, rejection = null) {
  updateTaskState(runtime.state, { phase: "waiting_response", last_phase: "waiting_response" });
  const start = Date.now();
  let lastHeartbeat = start;
  let lastKey = "";
  let stable = 0;
  while (Date.now() - start < MAX_WAIT_MS) {
    await sleep(STABLE_INTERVAL_MS);
    if (rejection?.code) throw new TaskFailure(rejection.code);
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
    await installDomCore(page);
    const [generating, count, text, images, files, errorCode, helperReady] = await page.evaluate(version => [
      window.__nyx?.isStillGenerating(),
      window.__nyx?.assistantCount(),
      window.__nyx?.extractResponse(),
      window.__nyx?.extractImages(),
      window.__nyx?.extractFiles(),
      window.__nyx?.errorCode(),
      window.__nyx?.version === version,
    ], DOM_CORE_VERSION);
    if (!helperReady) continue;
    if (errorCode) return recoverContentFailure(runtime, page, task, beforeCount, errorCode, rejection);
    const hasText = !!(text && text.length > 0);
    const hasImages = Array.isArray(images) && images.length > 0;
    const hasFiles = Array.isArray(files) && files.length > 0;
    // A text answer bumps the assistant-role count; an image-generation turn
    // does NOT (its <img> lives in a conversation-turn with no assistant role),
    // so artifacts carry that case through. Until text or an artifact appears
    // there's no new answer yet — wedge guard bails if ChatGPT has stopped.
    if (count <= beforeCount && !hasImages && !hasFiles) {
      if (!generating && Date.now() - start >= NO_OUTPUT_IDLE_MS) {
        return recoverContentFailure(runtime, page, task, beforeCount, "no_assistant_output", rejection);
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
        return recoverContentFailure(runtime, page, task, beforeCount, "no_assistant_output", rejection);
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
  // Never return an empty or partial response on the maximum-generation deadline.
  throw new TaskFailure('response_timeout');
}

async function recoverContentFailure(runtime, page, task, beforeCount, code, rejection = null) {
  // A rejected message was never delivered; a reload cannot make it appear.
  if (['usage_limit_reached', 'model_unavailable', 'prompt_too_long'].includes(code)) throw new TaskFailure(code);
  if (runtime.state.current_task?.content_reload_attempted) throw new TaskFailure(code);
  updateTaskState(runtime.state, { content_reload_attempted: true });
  await page.reload({ waitUntil: 'domcontentloaded', timeout: 60000 });
  await installDomCore(page);
  await waitForComposer(page, 15000);
  await settleDom(page, { quietMs: 250, maxMs: Math.max(1500, STABLE_INTERVAL_MS) });
  const snapshot = await transcriptSnapshot(page);
  const decision = decidePromptResume({ phase: runtime.state.current_task?.phase, prompt: task.prompt,
    turns: snapshot.turns, generating: snapshot.generating, transcriptReady: snapshot.ready,
    baselineTurnCount: runtime.state.current_task?.baseline_turn_count || 0 });
  if (snapshot.errorCode) throw new TaskFailure(snapshot.errorCode);
  if (decision.action === 'complete') return { text: decision.response, images: snapshot.images, files: snapshot.files };
  if (decision.action === 'wait') return waitForResponse(runtime, page, task, beforeCount, rejection);
  throw new TaskFailure(code);
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
      if (stableErrorCode(e) === "page_crashed") throw e;
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
      if (stableErrorCode(e) === "page_crashed") throw e;
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
  await settleDom(page, { quietMs: 250, maxMs: 1500 });

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
    if (!content) throw new TaskFailure("empty_extraction");
    const response = content;
    const res = await apiPost("/result", taskIdentity(runtime, task, {
      response,
      chatgpt_url: page.url(),
      model: task.model,
    }));
    log(`extract ${task_id} → ${res.status} (${content.length} chars)`);
  } catch (err) {
    if (stableErrorCode(err) === 'page_crashed') throw err;
    throw new TaskFailure(stableErrorCode(err));
  }
}

async function ack(runtime, task, phase, phaseDetail) {
  updateTaskState(runtime.state, { last_phase: phase });
  const response = await apiPost("/ack", taskIdentity(runtime, task, {
    phase,
    phase_detail: phaseDetail,
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
    cooldown_remaining_secs: cooldownRemaining(runtime.state.cooldown_until),
    logged_in: loggedIn,
    current_task_id: runtime.state.current_task?.task_id || null,
    chrome_alive: runtime.chromeAlive,
    last_error: runtime.lastError || runtime.state.last_task_failure || runtime.state.saved_login_error || null,
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
  const previousDetail = runtime.state.current_task?.failure_detail;
  const detail = /^[a-z0-9_]{1,64}(@[a-z0-9_]{1,40})?$/.test(previousDetail || "") ? previousDetail
    : failureDetail(code, runtime.state.current_task?.last_phase || runtime.state.current_task?.phase);
  runtime.lastError = detail;
  runtime.state.last_task_failure = detail;
  saveState(runtime.state);
  if (code === 'usage_limit_reached') {
    runtime.state.cooldown_until = Date.now() + USAGE_COOLDOWN_MS;
    saveState(runtime.state);
  }
  log(`task_failure code=${code} detail=${detail} probe=${JSON.stringify(await failureProbe(runtime.page))}`);
  await writeDiagnosticSnapshot(runtime, task, code, detail);
  await apiPost(
    "/result",
    taskIdentity(runtime, task, {
      response: `ERROR: ${code}`,
      failure_detail: detail,
      observed_model_switcher: runtime.state.current_task?.observed_model_switcher,
      observed_model_effort: runtime.state.current_task?.observed_model_effort,
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
      runtime.state.last_task_failure = null;
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
        const underlying = ['prompt_delivery_uncertain', 'recovery_conversation_unknown'].includes(code) && runtime.state.current_task?.failure_detail;
        updateTaskState(runtime.state, { failure_detail: underlying || failureDetail(code, runtime.state.current_task?.last_phase || runtime.state.current_task?.phase) });
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
      const cause = runtime.pageCrashed ? "page_crashed" : stableErrorCode(error);
      const detail = failureDetail(cause, runtime.state.current_task?.last_phase || runtime.state.current_task?.phase);
      const shapeFailure = ['composer_not_found', 'send_button_not_found', 'composer_readback_failed', 'composer_unobstructed_failed', 'chatgpt_error_response'].includes(cause)
        && (await failureProbe(runtime.page)).logged_in;
      const shapeFailures = (runtime.state.current_task?.shape_failures || 0) + Number(shapeFailure);
      updateTaskState(runtime.state, { recovery_failures: failureCount, failure_detail: detail, shape_failures: shapeFailures });
      runtime.lastError = detail;
      log(`task ${task.task_id} browser failure ${failureCount}/${MAX_TASK_RECOVERY_FAILURES} (${runtime.lastError})`);
      await writeDiagnosticSnapshot(runtime, task, cause, detail);
      const recovery = taskRecoveryDecision({
        kind: task.kind,
        phase: runtime.state.current_task?.phase,
        failureCount,
        shapeFailures,
      });
      runtime.chromeAlive = false;
      if (recovery.action === "fail") {
        runtime.lastError = recovery.code;
        await settleTaskFailure(runtime, task, recovery.code);
        clearTaskState(runtime.state);
        return;
      }
      runtime.health.cdp += 1;
      log(`task ${task.task_id} paused for browser recovery (${runtime.lastError})`);
      if (cause === 'page_crashed' && runtime.context) {
        runtime.pageCrashed = true;
        await replaceCrashedPage(runtime, runtime.state.current_task?.conversation_url || task.conversation_url);
      } else {
        await recoverChrome(runtime, shapeFailure || recovery.forceRelaunch);
      }
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
  if (USAGE_COOLDOWN.invalid) log("usage_cooldown_invalid default_seconds=900");
  await recoverChrome(runtime);

  for (;;) {
    try {
      if (Date.now() - runtime.lastPresenceAt >= PRESENCE_MS) await heartbeat(runtime);
      if (await processPendingCommand(runtime)) process.exit(75);
      await processSavedLogin(runtime);
      if ((runtime.state.draining || cooldownRemaining(runtime.state.cooldown_until) > 0) && !runtime.state.current_task) {
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
          `&instance_id=${encodeURIComponent(state.instance_id)}`
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
