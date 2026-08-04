// ==UserScript==
// @name         NyxID Oracle Worker
// @namespace    nyxid
// @version      1.0.0
// @description  NyxID browser oracle worker: serves ChatGPT Pro answers to NyxID oracle pools. Configurable server + worker token.
// @match        https://chatgpt.com/*
// @match        https://chat.openai.com/*
// @grant        GM_xmlhttpRequest
// @grant        GM_setValue
// @grant        GM_getValue
// @connect      *
// @run-at       document-idle
// @noframes
// ==/UserScript==
//
// NOTE on `@connect *`:
//   The NyxID server host is user-configured at runtime (nyxid_base_url in the
//   panel), so we cannot hardcode a specific @connect host the way the upstream
//   BEDC bridge pinned localhost/127.0.0.1. A wildcard @connect lets
//   GM_xmlhttpRequest reach whichever NyxID deployment the operator points at
//   (e.g. https://auth.nyxid.dev). All traffic still carries an Authorization
//   bearer worker token and only targets ${nyxid_base_url}/api/v1/oracle/worker.
//
// FORKED FROM: tools/bedc-deep/bedc_oracle_macos.user.js v1.21 (the BEDC oracle
// bridge). The entire DOM-automation core (prompt entry, send detection,
// response/ math extraction, PDF upload, per-tab GM storage) is preserved
// VERBATIM. Only the config + networking + identity + project-enforcement
// layer was rewritten to talk to NyxID over HTTPS with a pool worker token
// instead of a local no-auth python server.
//
// What changed vs BEDC:
//   - SERVER const → GM-stored config (nyxid_base_url + nyxid_worker_token),
//     editable from the panel. Worker API base = ${base}/api/v1/oracle/worker.
//   - Every request carries Authorization: Bearer <nyxid_worker_token>.
//   - URL opt-in flag ?bedc=N → ?nyx=N (worker label tab_N).
//   - Per-tab identity agentId() → workerLabel() (tab_<label> convention).
//   - Hardcoded BEDC project prefix → server-driven required_project_url,
//     cached between polls. No required_project_url ⇒ no project constraint.
//   - /task response: task fields are now SIBLINGS of status:"task" (not the
//     task dict directly). Poll loop branches on status === "task".
//   - Result/ack/pin payload field agent_id → worker.
//   - Panel branding: "NyxID Oracle" (purple/cyan retained).

(function () {
  "use strict";

  try {
    if (window.top !== window.self) return;
  } catch {
    return;
  }
  if (window.location.pathname.startsWith("/backend-api/")) return;
  if (window.location.href.includes("/sentinel/")) return;

  // ── Timing constants (verbatim from BEDC) ────────────────────────────
  const POLL_INTERVAL = 30000;
  const STABLE_CHECKS = 3;
  const STABLE_INTERVAL = 60000;
  const MAX_WAIT = 7200000;
  const NO_OUTPUT_IDLE_TIMEOUT = 420000;
  const REFILL_NO_OUTPUT_IDLE_TIMEOUT = 1800000;
  const SCRIPT_VERSION = "nyxid-1.0.0";

  // ── Configuration (GM storage; cross-tab is fine for config — only
  // task-state must stay per-tab via tabSet/tabGet) ────────────────────
  //   nyxid_base_url      e.g. https://auth.nyxid.dev (trailing slash stripped)
  //   nyxid_worker_token  pool worker token (nyx_owk_...)
  //   nyxid_worker_label  per-tab worker identity, default tab_1, ?nyx=N → tab_N
  const CFG_BASE_URL = "nyxid_base_url";
  const CFG_WORKER_TOKEN = "nyxid_worker_token";
  const CFG_WORKER_LABEL = "nyxid_worker_label";

  function stripTrailingSlash(s) {
    return (s || "").replace(/\/+$/, "");
  }
  function baseUrl() {
    try { return stripTrailingSlash(GM_getValue(CFG_BASE_URL, "") || ""); }
    catch { return ""; }
  }
  function workerToken() {
    try { return (GM_getValue(CFG_WORKER_TOKEN, "") || "").trim(); }
    catch { return ""; }
  }
  function workerApiBase() {
    const b = baseUrl();
    return b ? `${b}/api/v1/oracle/worker` : "";
  }
  function isConfigured() {
    return !!baseUrl() && !!workerToken();
  }

  // Default worker label (configurable; the ?nyx= URL flag overrides per tab).
  function defaultWorkerLabel() {
    try {
      const v = (GM_getValue(CFG_WORKER_LABEL, "") || "").trim();
      return v || "tab_1";
    } catch { return "tab_1"; }
  }

  // Cached required_project_url returned by the server. Null/empty ⇒ no project
  // constraint; operate on any chatgpt.com page. Updated on every poll/task
  // response so project enforcement works between polls.
  let requiredProjectUrl = "";
  function setRequiredProjectUrl(url) {
    const next = stripTrailingSlash(url || "");
    if (next !== requiredProjectUrl) {
      requiredProjectUrl = next;
      log(next
        ? `Pool requires project: ${next.slice(-60)}`
        : "Pool has no project constraint");
    }
  }
  // Derive the path prefix of the required project for the "are we inside it"
  // check. e.g. https://chatgpt.com/g/g-p-XXXX-name/project → /g/g-p-XXXX-name
  function requiredProjectPathPrefix() {
    if (!requiredProjectUrl) return "";
    try {
      const u = new URL(requiredProjectUrl, window.location.origin);
      // Prefer the /g/<slug> namespace if present.
      const g = u.pathname.match(/^\/g\/[^/]+/);
      if (g) return g[0];
      // Otherwise strip a trailing /project segment, else use the path as-is.
      return u.pathname.replace(/\/project\/?$/, "").replace(/\/+$/, "") || "/";
    } catch { return ""; }
  }
  function requiredProjectHome() {
    if (!requiredProjectUrl) return "";
    try {
      const u = new URL(requiredProjectUrl, window.location.origin);
      // If the configured URL already looks like a project home, keep it.
      if (/\/project\/?$/.test(u.pathname)) return stripTrailingSlash(u.toString());
      const prefix = requiredProjectPathPrefix();
      if (prefix && prefix.startsWith("/g/")) {
        return `${u.origin}${prefix}/project`;
      }
      return stripTrailingSlash(u.toString());
    } catch { return requiredProjectUrl; }
  }

  // If a required project is set: are we inside it? If none set: always "true"
  // (any page is acceptable), so project enforcement becomes a no-op.
  function isInsideRequiredProject() {
    if (!requiredProjectUrl) return true;
    const prefix = requiredProjectPathPrefix();
    if (!prefix || prefix === "/") return true;
    return window.location.pathname.startsWith(prefix);
  }

  function nyxFlagFromUrl() {
    const m = window.location.search.match(/[?&]nyx=([^&]+)/);
    return m ? m[1] : "";
  }

  // Project home URL with this tab's nyx= flag pinned (for navigate-back).
  function projectEntryUrl() {
    const home = requiredProjectHome();
    if (!home) return "";
    const flag = workerLabelFlag();
    return `${home}?nyx=${encodeURIComponent(flag)}`;
  }

  let busy = false;
  // Per-tab active flag via sessionStorage (NOT GM_setValue, which is cross-tab
  // and caused new ChatGPT windows the user opens for personal use to inherit
  // ACTIVE state and start stealing tasks). Each tab opts in independently via
  // a ?nyx= URL or the dashboard toggle.
  let active = (() => {
    const urlOptIn = window.location.search.includes("nyx=");
    try {
      if (urlOptIn) sessionStorage.setItem("nyxid_active", "1");
      const storedActive = sessionStorage.getItem("nyxid_active") === "1";
      // With a required project, only count active inside it (unless URL opt-in
      // forces it for this load); with no required project, any page counts.
      return storedActive && (urlOptIn || isInsideRequiredProject());
    } catch {
      return urlOptIn;
    }
  })();

  // ── Logging ──────────────────────────────────────────────────────────
  const logHistory = [];
  function log(msg) {
    const ts = new Date().toLocaleTimeString();
    const entry = `${ts} ${msg}`;
    console.log(`[nyxid] ${entry}`);
    logHistory.push(entry);
    if (logHistory.length > 20) logHistory.shift();
    updatePanel();
  }

  function toggleActive() {
    if (!isConfigured()) {
      log("Cannot start — configure base URL + worker token first");
      openConfigForm();
      return;
    }
    active = !active;
    try { sessionStorage.setItem("nyxid_active", active ? "1" : "0"); } catch {}
    log(active ? "ACTIVATED — polling will start (this tab only)" : "PAUSED — your ChatGPT is free");
    updatePanel();
  }

  // ── Status panel (NyxID branding; purple/cyan) ──────────────────────
  let panel = null;
  function ensurePanel() {
    if (panel && document.body.contains(panel)) return;
    panel = document.createElement("div");
    panel.id = "nyxid-oracle-panel";
    panel.style.cssText = `
      position: fixed; bottom: 12px; right: 12px; z-index: 99999;
      background: #1d1d3a; color: #9af; font-family: monospace; font-size: 11px;
      padding: 8px 12px; border-radius: 6px; max-width: 460px; max-height: 360px;
      overflow-y: auto; box-shadow: 0 2px 12px rgba(80,40,180,0.5); opacity: 0.93;
      line-height: 1.4; border: 1px solid #5577cc;
    `;
    document.body.appendChild(panel);
  }

  function openConfigForm() {
    showConfigForm = true;
    updatePanel();
  }
  function closeConfigForm() {
    showConfigForm = false;
    updatePanel();
  }
  let showConfigForm = false;

  function saveConfigFromForm() {
    try {
      const baseEl = document.getElementById("nyxid-cfg-base");
      const tokEl = document.getElementById("nyxid-cfg-token");
      const labelEl = document.getElementById("nyxid-cfg-label");
      if (baseEl) GM_setValue(CFG_BASE_URL, stripTrailingSlash((baseEl.value || "").trim()));
      if (tokEl) GM_setValue(CFG_WORKER_TOKEN, (tokEl.value || "").trim());
      if (labelEl) {
        const lbl = (labelEl.value || "").trim();
        if (lbl) GM_setValue(CFG_WORKER_LABEL, lbl);
      }
      log(`Config saved (base=${baseUrl() || "?"}, token=${workerToken() ? "set" : "missing"})`);
    } catch (e) {
      log(`Config save failed: ${e.message}`);
    }
    showConfigForm = false;
    updatePanel();
  }

  function updatePanel() {
    ensurePanel();
    const configured = isConfigured();
    const statusColor = !configured ? "#fa0" : (active ? (busy ? "#ff0" : "#9af") : "#f55");
    const statusText = !configured ? "NOT CONFIGURED" : (active ? (busy ? "BUSY" : "ACTIVE") : "PAUSED");
    const btnText = active ? "⏸ Pause" : "▶ Start";
    const btnColor = active ? "#f55" : "#9af";
    const lines = logHistory.slice(-10).map(l => `<div>${l}</div>`).join("");
    const baseDisplay = configured ? baseUrl() : "⚠ not configured";
    const lbl = workerLabel();

    if (showConfigForm) {
      panel.innerHTML = `
        <div style="display:flex;justify-content:space-between;align-items:center;gap:8px">
          <b style="color:#cdf">[NyxID Oracle — Settings]</b>
          <button id="nyxid-cfg-close" style="background:#446;color:#cdf;border:none;border-radius:3px;padding:2px 8px;cursor:pointer;font-size:11px">✕</button>
        </div>
        <hr style="border-color:#446;margin:4px 0">
        <div style="display:flex;flex-direction:column;gap:6px">
          <label style="color:#9bd">Base URL (e.g. https://auth.nyxid.dev)
            <input id="nyxid-cfg-base" type="text" value="${escapeAttr(baseUrl())}"
              style="width:100%;box-sizing:border-box;background:#0d0d22;color:#cdf;border:1px solid #5577cc;border-radius:3px;padding:3px;font-family:monospace;font-size:11px" />
          </label>
          <label style="color:#9bd">Worker token (nyx_owk_...)
            <input id="nyxid-cfg-token" type="password" value="${escapeAttr(workerToken())}"
              style="width:100%;box-sizing:border-box;background:#0d0d22;color:#cdf;border:1px solid #5577cc;border-radius:3px;padding:3px;font-family:monospace;font-size:11px" />
          </label>
          <label style="color:#9bd">Worker label (tab id; ?nyx=N overrides)
            <input id="nyxid-cfg-label" type="text" value="${escapeAttr(defaultWorkerLabel())}"
              style="width:100%;box-sizing:border-box;background:#0d0d22;color:#cdf;border:1px solid #5577cc;border-radius:3px;padding:3px;font-family:monospace;font-size:11px" />
          </label>
          <button id="nyxid-cfg-save" style="background:#9af;color:#000;border:none;border-radius:3px;padding:4px 8px;cursor:pointer;font-size:11px;font-weight:bold">Save</button>
        </div>`;
      const closeBtn = document.getElementById("nyxid-cfg-close");
      if (closeBtn) closeBtn.addEventListener("click", closeConfigForm);
      const saveBtn = document.getElementById("nyxid-cfg-save");
      if (saveBtn) saveBtn.addEventListener("click", saveConfigFromForm);
      return;
    }

    panel.innerHTML = `
      <div style="display:flex;justify-content:space-between;align-items:center;gap:8px">
        <b style="color:#cdf">[NyxID Oracle ${SCRIPT_VERSION}]</b>
        <span style="color:${statusColor};font-weight:bold">${statusText}</span>
        ${configured
          ? `<button id="nyxid-toggle" style="background:${btnColor};color:#000;border:none;border-radius:3px;padding:2px 8px;cursor:pointer;font-size:11px;font-weight:bold">${btnText}</button>`
          : `<button id="nyxid-configure" style="background:#fa0;color:#000;border:none;border-radius:3px;padding:2px 8px;cursor:pointer;font-size:11px;font-weight:bold">Configure</button>`}
      </div>
      <div style="color:#79b;margin:2px 0">server: ${escapeHtml(baseDisplay)} · worker: ${escapeHtml(lbl)}</div>
      <div style="display:flex;justify-content:flex-end;gap:6px;margin-bottom:2px">
        <button id="nyxid-settings" style="background:#446;color:#cdf;border:none;border-radius:3px;padding:1px 6px;cursor:pointer;font-size:10px">⚙ Settings</button>
      </div>
      <hr style="border-color:#446;margin:4px 0">
      ${lines}`;
    const btn = document.getElementById("nyxid-toggle");
    if (btn) btn.addEventListener("click", toggleActive);
    const cfgBtn = document.getElementById("nyxid-configure");
    if (cfgBtn) cfgBtn.addEventListener("click", openConfigForm);
    const setBtn = document.getElementById("nyxid-settings");
    if (setBtn) setBtn.addEventListener("click", openConfigForm);
  }

  function escapeHtml(s) {
    return String(s || "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }
  function escapeAttr(s) {
    return String(s || "").replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  // ── HTTP helpers (rewritten for NyxID bearer auth) ───────────────────
  // Base = ${baseUrl()}/api/v1/oracle/worker. Every request carries
  // Authorization: Bearer <workerToken()> + Content-Type: application/json.
  // page_url + script_version are still appended (accepted as optional);
  // the BEDC chatgpt_url auto-injection on GET is dropped (not in NyxID spec).
  function serverGet(path) {
    return new Promise((resolve, reject) => {
      const apiBase = workerApiBase();
      if (!apiBase) { reject(new Error("not configured")); return; }
      const sep = path.includes("?") ? "&" : "?";
      const meta = `${sep}script_version=${encodeURIComponent(SCRIPT_VERSION)}`
        + `&page_url=${encodeURIComponent(window.location.href)}`;
      GM_xmlhttpRequest({
        method: "GET",
        url: `${apiBase}${path}${meta}`,
        headers: {
          "Authorization": `Bearer ${workerToken()}`,
          "Content-Type": "application/json",
        },
        timeout: 10000,
        onload: (r) => {
          try { resolve(JSON.parse(r.responseText)); }
          catch (e) { reject(e); }
        },
        onerror: () => reject(new Error("network error")),
        ontimeout: () => reject(new Error("timeout")),
      });
    });
  }

  function serverPost(path, data) {
    return new Promise((resolve, reject) => {
      const apiBase = workerApiBase();
      if (!apiBase) { reject(new Error("not configured")); return; }
      // Keep page_url + script_version (optional, still useful). Drop BEDC's
      // chatgpt_url auto-injection — the NyxID payloads name it per-endpoint.
      const payload = Object.assign({}, data || {}, {
        script_version: SCRIPT_VERSION,
        page_url: window.location.href,
      });
      GM_xmlhttpRequest({
        method: "POST",
        url: `${apiBase}${path}`,
        headers: {
          "Authorization": `Bearer ${workerToken()}`,
          "Content-Type": "application/json",
        },
        data: JSON.stringify(payload),
        timeout: 30000,
        onload: (r) => {
          try { resolve(JSON.parse(r.responseText)); }
          catch (e) { reject(e); }
        },
        onerror: () => reject(new Error("network error")),
        ontimeout: () => reject(new Error("timeout")),
      });
    });
  }

  function sleep(ms) {
    return new Promise((r) => setTimeout(r, ms));
  }

  // ── Persistent task state (survives page navigation) ─────────────────
  // Keys namespaced per-tab via tabSet/tabGet (scoped by workerLabel()).
  // in_flight_task_id tracks the currently-being-processed task across full
  // page reloads (ChatGPT does a full reload on the first /c/<uuid> redirect,
  // dropping in-memory busy state). With in_flight set, a re-entry of
  // processTask while on the original /c/<uuid> page resumes waitForResponse()
  // instead of re-navigating + re-entering the prompt. Scoping every key by
  // workerLabel() keeps tabs from trampling each other's state.
  function saveTaskState(task) {
    tabSet("current_task", JSON.stringify(task));
    tabSet("task_phase", "pending");
  }
  function loadTaskState() {
    try {
      const s = tabGet("current_task", "");
      return s ? JSON.parse(s) : null;
    } catch { return null; }
  }
  function getTaskPhase() {
    return tabGet("task_phase", "");
  }
  function setTaskPhase(phase) {
    tabSet("task_phase", phase);
  }
  function clearTaskState() {
    tabSet("current_task", "");
    tabSet("task_phase", "");
  }
  function getInFlightTaskId() {
    return tabGet("in_flight_task_id", "");
  }
  function setInFlightTaskId(id) {
    tabSet("in_flight_task_id", id || "");
    // Also stamp the URL we were on when we became busy with this task.
    if (id) {
      tabSet("in_flight_url", window.location.href);
      tabSet("in_flight_started_at", Date.now());
    } else {
      tabSet("in_flight_url", "");
      tabSet("in_flight_started_at", 0);
    }
  }
  function getInFlightUrl() {
    return tabGet("in_flight_url", "");
  }
  function getInFlightAgeMs() {
    const ts = tabGet("in_flight_started_at", 0);
    return ts ? (Date.now() - ts) : 0;
  }

  // ── DOM helpers (verbatim from BEDC / paper oracle v4.10) ────────────

  function findPromptInput() {
    for (const sel of [
      "#prompt-textarea",
      "div.ProseMirror[contenteditable='true']",
      "[id='prompt-textarea']",
      "div[contenteditable='true'][role='textbox']",
      "div[contenteditable='true']",
    ]) {
      const el = document.querySelector(sel);
      if (el) return el;
    }
    return null;
  }

  function findFileInput() {
    // ChatGPT has a hidden file input on the composer
    return document.querySelector("input[type='file']");
  }

  // ── PDF upload (verbatim) ────────────────────────────────────────────
  async function waitForUploadComplete(timeoutMs = 60000) {
    log("Waiting for PDF upload to complete...");
    const start = Date.now();
    while (Date.now() - start < timeoutMs) {
      await sleep(2000);
      const uploading =
        document.querySelector("[class*='uploading']") ||
        document.querySelector("[class*='progress']") ||
        document.querySelector("[role='progressbar']") ||
        document.querySelector("[class*='loading']");
      const attached =
        document.querySelector("[class*='attachment']") ||
        document.querySelector("[class*='file-chip']") ||
        document.querySelector("[data-testid*='attachment']") ||
        document.querySelector("[class*='uploaded']") ||
        document.querySelector("img[alt*='pdf']") ||
        document.querySelector("[class*='file']");
      const elapsed = Math.floor((Date.now() - start) / 1000);
      if (!uploading && attached) {
        log(`PDF upload complete (${elapsed}s), attachment visible`);
        return true;
      }
      const sendBtn = findSendButton();
      if (sendBtn && !sendBtn.disabled && elapsed > 5) {
        log(`PDF upload likely complete (${elapsed}s), send button enabled`);
        return true;
      }
      if (elapsed % 10 === 0 && elapsed > 0) {
        log(`Upload waiting... ${elapsed}s (uploading=${!!uploading}, attached=${!!attached})`);
      }
    }
    log("Upload wait timeout — proceeding anyway");
    return false;
  }

  async function uploadPDF(base64Data, fileName) {
    log(`PDF upload: ${fileName} (${(base64Data.length * 0.75 / 1024).toFixed(0)} KB)`);
    const byteChars = atob(base64Data);
    const byteArray = new Uint8Array(byteChars.length);
    for (let i = 0; i < byteChars.length; i++) byteArray[i] = byteChars.charCodeAt(i);
    const file = new File([byteArray], fileName, { type: "application/pdf" });

    let injected = false;

    // Method 1: hidden file input
    const fileInput = findFileInput();
    if (fileInput) {
      try {
        const dt = new DataTransfer();
        dt.items.add(file);
        fileInput.files = dt.files;
        fileInput.dispatchEvent(new Event("change", { bubbles: true }));
        log("PDF: injected via file input");
        injected = true;
      } catch (e) {
        log(`PDF file input failed: ${e.message}`);
      }
    }

    // Method 2: click attach button, then file input
    if (!injected) {
      const attachBtn = document.querySelector(
        "button[aria-label='Attach files'], button[aria-label='Upload file'], " +
        "button[data-testid='composer-attach-button'], button[aria-haspopup='menu']"
      );
      if (attachBtn) {
        log("PDF: clicking attach button...");
        attachBtn.click();
        await sleep(1000);
        const fi2 = document.querySelector("input[type='file']");
        if (fi2) {
          try {
            const dt2 = new DataTransfer();
            dt2.items.add(file);
            fi2.files = dt2.files;
            fi2.dispatchEvent(new Event("change", { bubbles: true }));
            log("PDF: injected after clicking attach");
            injected = true;
          } catch (e) {
            log(`PDF inject after attach failed: ${e.message}`);
          }
        }
      }
    }

    // Method 3: drag-drop on composer
    if (!injected) {
      log("PDF: trying drag-drop...");
      const dropTarget =
        document.querySelector("form") ||
        findPromptInput()?.closest("div") ||
        document.querySelector("[class*='composer']");
      if (dropTarget) {
        const dt3 = new DataTransfer();
        dt3.items.add(file);
        for (const evtType of ["dragenter", "dragover", "drop"]) {
          dropTarget.dispatchEvent(new DragEvent(evtType, {
            bubbles: true, cancelable: true, dataTransfer: dt3,
          }));
          await sleep(300);
        }
        log("PDF: drag-drop dispatched");
        injected = true;
      }
    }

    if (!injected) {
      log("PDF: ALL METHODS FAILED — continuing without PDF");
      return false;
    }
    await waitForUploadComplete(60000);
    return true;
  }

  function findSendButton(allowDisabled = false) {
    for (const sel of [
      "button[data-testid='send-button']",
      "button[data-testid='composer-send-button']",
      "button[aria-label='Send prompt']",
      "button[aria-label='发送提示']",
      "button[aria-label='Send']",
      "button[aria-label='Send message']",
      "button[aria-label='Submit']",
    ]) {
      const el = document.querySelector(sel);
      if (el && (allowDisabled || !el.disabled)) return el;
    }
    for (const btn of document.querySelectorAll("button[data-testid]")) {
      const tid = btn.getAttribute("data-testid") || "";
      if (tid.toLowerCase().includes("send") && (allowDisabled || !btn.disabled)) return btn;
    }
    function isNonSendButton(btn) {
      const tid = (btn.getAttribute("data-testid") || "").toLowerCase();
      const label = (btn.getAttribute("aria-label") || "").toLowerCase();
      const text = (btn.textContent || "").toLowerCase();
      const all = tid + " " + label + " " + text;
      return /plus|attach|file|添加|文件|mic|voice|听写|dictation|new|model|专业|search|搜索/.test(all);
    }
    const formAreas = [
      document.querySelector("form"),
      document.querySelector("[class*='composer']"),
      document.querySelector("[class*='input-area']"),
      document.querySelector("[class*='prompt']")?.closest("div[class]"),
    ].filter(Boolean);
    for (const area of formAreas) {
      for (const btn of area.querySelectorAll("button:not([disabled])")) {
        if (isNonSendButton(btn)) continue;
        const svg = btn.querySelector("svg");
        if (svg) {
          const paths = svg.querySelectorAll("path, polyline, line");
          if (paths.length > 0 && paths.length < 5) {
            const rect = btn.getBoundingClientRect();
            if (rect.width > 0 && rect.height > 0) return btn;
          }
        }
      }
    }
    const promptInput = findPromptInput();
    if (promptInput) {
      let container = promptInput.parentElement;
      for (let depth = 0; depth < 6 && container; depth++) {
        const btns = container.querySelectorAll("button:not([disabled])");
        for (const btn of btns) {
          if (isNonSendButton(btn)) continue;
          if (btn.querySelector("svg")) return btn;
        }
        container = container.parentElement;
      }
    }
    return null;
  }

  function isOnNewChatPage() {
    const msgs = document.querySelectorAll("[data-message-author-role]");
    return msgs.length === 0;
  }

  // Force a /c/<id> URL into the required-project namespace. Idempotent: a URL
  // already inside a /g/<slug>/ namespace is returned unchanged. If NO required
  // project is configured, returns the URL untouched (no constraint).
  function pinToProject(url) {
    if (!url) return url;
    if (!requiredProjectUrl) return url; // no project constraint — pass through
    const prefix = requiredProjectPathPrefix();
    if (!prefix || prefix === "/") return url;
    try {
      const u = new URL(url, window.location.origin);
      // already inside any /g/<slug>/ namespace — trust it
      if (/^\/g\/[^/]+\//.test(u.pathname)) return u.toString();
      // bare /c/<id> — splice in the required project prefix
      const m = u.pathname.match(/^\/c\/[a-f0-9-]{6,}/);
      if (m) {
        u.pathname = `${prefix}${m[0]}`;
        return u.toString();
      }
      // bare root or other path — return as-is, caller decides
      return u.toString();
    } catch {
      return url;
    }
  }

  // Detect if current page is a /c/<id> conversation. Returns a project-pinned
  // URL (when a project is required) so the server doesn't store a bare /c/<id>
  // that would later leak the tab out of the project. With no required project,
  // returns the bare conversation URL.
  function currentChatUrl() {
    const href = window.location.href;
    if (/\/c\/[a-f0-9-]{6,}/.test(href)) {
      return pinToProject(href.split("?")[0]);
    }
    return "";
  }

  // If a project IS required and we've drifted out of it (URL is /c/<id> with
  // no /g/<slug> prefix), redirect back into the project namespace before doing
  // anything that depends on project context. Returns true if we navigated.
  // No-op when no project is required.
  function ensureInProject() {
    if (!requiredProjectUrl) return false;
    const href = window.location.href;
    if (!/\/c\/[a-f0-9-]{6,}/.test(href)) return false;
    if (isInsideRequiredProject()) return false;
    const pinned = pinToProject(href);
    if (pinned !== href) {
      log(`drift detected — navigating ${href.slice(-40)} → project-pinned`);
      window.location.href = pinned;
      return true;
    }
    return false;
  }

  // ChatGPT redirects /?nyx=1 → /c/<latest>, so URL navigation to fresh chat
  // fails. Click the in-page "New Chat" button instead; this is an SPA
  // transition the app handles cleanly.
  function findNewChatButton() {
    for (const sel of [
      "button[data-testid='new-chat-button']",
      "a[data-testid='new-chat-button']",
      "button[aria-label='New chat']",
      "button[aria-label='New Chat']",
      "button[aria-label='新聊天']",
      "button[aria-label='新建聊天']",
      "a[href='/']",
      "a[href='/?model=']",
      "[data-testid='create-new-chat-button']",
      "[data-testid='create-new-chat']",
    ]) {
      const el = document.querySelector(sel);
      if (el) return el;
    }
    // Fallback: scan sidebar for an element with text content "New chat" / "新聊天"
    for (const el of document.querySelectorAll("a, button")) {
      const txt = (el.textContent || "").trim();
      if (/^(\+\s*)?(New chat|New Chat|新聊天|新建聊天)$/i.test(txt)) return el;
    }
    return null;
  }

  async function clickNewChatButton() {
    const btn = findNewChatButton();
    if (!btn) return false;
    btn.click();
    await sleep(2000); // SPA transition settle
    // Verify we landed on fresh chat
    return isOnNewChatPage();
  }

  // ── Enter prompt (verbatim) ──────────────────────────────────────────
  async function enterPrompt(text) {
    log(`Entering prompt (${text.length} chars)...`);
    const input = findPromptInput();
    if (!input) { log("ERROR: prompt input not found"); return false; }
    input.focus();
    await sleep(300);
    document.execCommand("selectAll", false, null);
    document.execCommand("delete", false, null);
    await sleep(200);
    let success = false;
    try {
      input.focus();
      const CHUNK = 4000;
      if (text.length <= CHUNK) {
        document.execCommand("insertText", false, text);
      } else {
        for (let i = 0; i < text.length; i += CHUNK) {
          document.execCommand("insertText", false, text.slice(i, i + CHUNK));
          await sleep(50);
        }
      }
      await sleep(500);
      if ((input.textContent || "").length > 10) {
        success = true;
        log("Prompt: inserted via execCommand (ProseMirror-native)");
      }
    } catch (e) { log(`execCommand failed: ${e.message}`); }
    if (!success) {
      try {
        await navigator.clipboard.writeText(text);
        input.focus();
        document.execCommand("paste");
        await sleep(500);
        if ((input.textContent || "").length > 10) {
          success = true;
          log("Prompt: pasted via clipboard API");
        }
      } catch (e) { log(`Clipboard paste failed: ${e.message}`); }
    }
    if (!success) {
      try {
        const clipData = new DataTransfer();
        clipData.setData("text/plain", text);
        input.dispatchEvent(new ClipboardEvent("paste", {
          bubbles: true, cancelable: true, clipboardData: clipData,
        }));
        await sleep(500);
        if ((input.textContent || "").length > 10) {
          success = true;
          log("Prompt: pasted via synthetic ClipboardEvent");
        }
      } catch (e) { log(`Synthetic paste failed: ${e.message}`); }
    }
    if (!success) {
      const escaped = text.replace(/&/g, "&amp;").replace(/</g, "&lt;")
                          .replace(/>/g, "&gt;").replace(/\n/g, "<br>");
      input.innerHTML = `<p>${escaped}</p>`;
      input.dispatchEvent(new InputEvent("input", {
        bubbles: true, cancelable: true, inputType: "insertText", data: text,
      }));
      input.dispatchEvent(new Event("change", { bubbles: true }));
      await sleep(500);
      log("Prompt: set via innerHTML (last resort)");
      success = (input.textContent || "").length > 0;
    }
    if (success) {
      input.focus();
      const sel = window.getSelection();
      if (sel) { sel.selectAllChildren(input); sel.collapseToEnd(); }
      document.execCommand("insertText", false, " ");
      await sleep(300);
      document.execCommand("delete", false, null);
      await sleep(300);
      log("Prompt: forced React sync (space+delete)");
    }
    const visible = (input.textContent || "").length;
    log(`Prompt visible: ${visible} chars, success=${success}`);
    return success;
  }

  // ── Click send (verbatim) ────────────────────────────────────────────
  async function clickSend() {
    await sleep(1000);
    log("Waiting for send button to be ready...");
    for (let i = 0; i < 60; i++) {
      const btn = findSendButton();
      if (btn && !btn.disabled) {
        const tid = btn.getAttribute("data-testid");
        const lbl = btn.getAttribute("aria-label");
        // Snapshot assistant message count IMMEDIATELY before send, so
        // waitForResponse can require count strict-increase. Structural fix for
        // duplicate-response (turn N capturing turn N-1's content because the
        // DOM still has prior turns visible).
        const pre = snapshotAssistantCount();
        log(`Send button found (testid=${tid}, label=${lbl}), pre-send assistant count=${pre}, clicking ONCE...`);
        btn.click();
        await sleep(500);
        return true;
      }
      if (i > 0 && i % 5 === 0) {
        const inp = findPromptInput();
        if (inp) {
          inp.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
          inp.dispatchEvent(new Event("change", { bubbles: true }));
        }
        log(`Send button still disabled... ${i * 0.5}s, retrying input event`);
      }
      await sleep(500);
    }
    const disabledSend = findSendButton(true);
    if (disabledSend) {
      log(`Force-clicking disabled send button`);
      disabledSend.disabled = false;
      disabledSend.removeAttribute("disabled");
      await sleep(100);
      disabledSend.click();
      await sleep(500);
      const inp = findPromptInput();
      const promptCleared = !inp || (inp.textContent || "").trim().length < 10;
      const stopBtn = document.querySelector("button[data-testid='stop-button']");
      if (promptCleared || stopBtn) { log("Force-click appears to have worked"); return true; }
    }
    const input = findPromptInput();
    if (input) {
      log("Send: trying Enter key...");
      input.focus();
      await sleep(100);
      for (const evtType of ["keydown", "keypress", "keyup"]) {
        input.dispatchEvent(new KeyboardEvent(evtType, {
          key: "Enter", code: "Enter", keyCode: 13, which: 13,
          bubbles: true, cancelable: true,
        }));
        await sleep(50);
      }
      await sleep(500);
      const remaining = (input.textContent || "").trim();
      if (remaining.length < 10) {
        log("Send: Enter key worked (prompt cleared)");
        return true;
      }
    }
    log("ERROR: cannot send after retries");
    return false;
  }

  // ── Response extraction (verbatim from paper oracle v4.10) ───────────
  let sentPromptText = "";
  let postSendLines = new Set();
  let lastBottomScrollAt = 0;
  let lastBottomScrollLogAt = 0;
  // Snapshot of `[data-message-author-role='assistant']` COUNT taken
  // immediately before we hit Send. waitForResponse waits until the count
  // strictly increases, then captures only the NEW last assistant message.
  // Without this, follow-up turns can return turn N-1's text because the
  // multi-strategy fallbacks in extractResponseText see prior assistant
  // messages still in DOM and pick one of them as "stable".
  let preSubmitAssistantCount = 0;
  function snapshotAssistantCount() {
    try {
      preSubmitAssistantCount = document.querySelectorAll(
        "[data-message-author-role='assistant']"
      ).length;
    } catch {
      preSubmitAssistantCount = 0;
    }
    return preSubmitAssistantCount;
  }
  function newAssistantCount() {
    try {
      return document.querySelectorAll("[data-message-author-role='assistant']").length;
    } catch {
      return 0;
    }
  }

  // ChatGPT can virtualize / lazily mount the latest assistant turn unless the
  // conversation viewport is near the bottom. Keep extraction pinned to the
  // newest rendered turn without weakening the count/stability gates below.
  function scrollConversationToBottom(reason = "", force = false) {
    const now = Date.now();
    if (!force && now - lastBottomScrollAt < 5000) return false;
    lastBottomScrollAt = now;
    try {
      const main = document.querySelector("main");
      const messageNodes = main
        ? main.querySelectorAll("[data-message-author-role]")
        : [];
      const lastMessage = messageNodes.length ? messageNodes[messageNodes.length - 1] : null;
      const scrollables = [
        document.scrollingElement,
        document.documentElement,
        document.body,
        main,
      ];
      if (main) {
        for (const el of Array.from(main.querySelectorAll("div, section, article")).slice(-120)) {
          try {
            const style = window.getComputedStyle(el);
            if (/(auto|scroll)/.test(style.overflowY || "")
                && el.scrollHeight > el.clientHeight + 40) {
              scrollables.push(el);
            }
          } catch {}
        }
      }
      const seen = new Set();
      for (const el of scrollables) {
        if (!el || seen.has(el)) continue;
        seen.add(el);
        try { el.scrollTop = el.scrollHeight; } catch {}
      }
      try {
        window.scrollTo({ top: document.body.scrollHeight, behavior: "auto" });
      } catch {
        try { window.scrollTo(0, document.body.scrollHeight); } catch {}
      }
      try {
        if (lastMessage) lastMessage.scrollIntoView({ block: "end", inline: "nearest", behavior: "auto" });
      } catch {}
      for (const el of seen) {
        try { el.scrollTop = el.scrollHeight; } catch {}
      }
      if (now - lastBottomScrollLogAt >= 300000) {
        lastBottomScrollLogAt = now;
        log(`Viewport pinned to latest response${reason ? ` (${reason})` : ""}`);
      }
      return true;
    } catch {
      return false;
    }
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
      } catch {}
      el = el.parentElement;
    }
    return document.scrollingElement || document.body;
  }

  function setSentPrompt(text) { sentPromptText = text; }

  function looksLikePromptEcho(text) {
    if (!sentPromptText || sentPromptText.length < 20) return false;
    const t = text.trim();
    if (/^(你说|You said)/i.test(t)) return true;
    const stripped = t
      .replace(/^(你说|You said)[：:]?\s*/i, "")
      .replace(/^main(\.pdf)?\s*/i, "")
      .replace(/^PDF\s*/i, "")
      .trim();
    const promptStart = sentPromptText.slice(0, 80).trim();
    if (stripped.length > 0
        && stripped.startsWith(promptStart)
        && stripped.length <= sentPromptText.length * 1.1) {
      return true;
    }
    if (t.length > 50
        && t.length < sentPromptText.length * 1.3
        && t.length > sentPromptText.length * 0.7) {
      const chunks = sentPromptText.match(/.{50}/g) || [];
      if (chunks.length > 10) {
        const hits = chunks.filter(c => t.includes(c)).length;
        if (hits / chunks.length > 0.8) return true;
      }
    }
    return false;
  }

  function capturePostSendState() {
    const main = document.querySelector("main");
    const text = main ? (main.innerText || "").trim() : "";
    postSendLines = new Set(text.split("\n").map(l => l.trim()).filter(l => l.length > 0));
    log(`Post-send captured: ${postSendLines.size} lines`);
  }

  function postSendNovelText(text) {
    if (postSendLines.size === 0) return "";
    const newLines = cleanText(text).split("\n").filter(l => {
      const t = l.trim();
      return t.length > 0 && !postSendLines.has(t) && !isChromeLine(t);
    });
    if (newLines.length < 2) return "";
    const joined = newLines.join("\n").trim();
    return joined.length >= 100 ? joined : "";
  }

  function hasPostSendNovelContent(text) {
    if (postSendLines.size === 0) return true;
    return postSendNovelText(text).length >= 100;
  }

  const CHROME_RE = [
    /^(进阶专业|ChatGPT\s*也可能会犯错|请核查重要信息|查看\s*Cookie|Cookie\s*首选项)/,
    /^(ChatGPT can make mistakes|Check important info)/,
    /^Extended\s*Pro$/i,
    /^(Deep research|Deep thinking|Reasoning)$/i,
    /^Thought for \d+/,
    /^(你说|You said|ChatGPT\s*说|ChatGPT\s*said)[：:]?\s*$/,
    /^(正在思考|正在搜索|Searching)/,
    /^main(\.pdf)?\s*$/,
    /^PDF\s*$/,
    /^(进阶专业模式|click to remove|Start dictation|Send prompt)/,
    /^(新建聊天|New chat|搜索聊天|Search chats|图片|Images)/,
    /^(查看方案|See plans|设置|Settings|帮助|Help)/,
    /^(获取根据保存的聊天量身定制的回复|Get responses tailored)/,
    /^(登录|Log in|注册|Sign up)/,
    /^(我们使用\s*cookie|We use cookies|管理\s*Cookie|Manage Cookies)/,
    /^(拒绝非必要|Reject non-essential|接受所有|Accept all)/,
    /^See Cookie Preferences/,
  ];
  // SSR markers were being matched against the WHOLE extracted response,
  // causing any review that contained those strings (e.g. a critique of our own
  // code) to be rejected forever. Now: cleanText strips SSR boot lines first,
  // isSSRGarbage only fires for SHORT responses.
  const SSR_GARBAGE_RE = /window\.__oai_log|window\.__oai_SSR|requestAnimationFrame/;
  const SSR_LINE_RE = /window\.__oai_(log|SSR)\s*[=(]|requestAnimationFrame\s*\(/;

  function isChromeLine(t) {
    if (!t || t.length > 200) return false;
    return CHROME_RE.some(re => re.test(t));
  }

  function stableResponseKey(text) {
    const normalized = cleanText(text)
      .replace(/Thought for\s+\d+(?:\s*m\s*\d+\s*s|\s*s|\s+min)/gi, "Thought for <elapsed>")
      .replace(/\b\d{1,2}:\d{2}(?::\d{2})?\b/g, "<clock>")
      .replace(/\b(Pro thinking|Extended Pro|Reasoning…)\b/gi, "")
      .replace(/[ \t]+/g, " ")
      .replace(/\n{3,}/g, "\n\n")
      .trim();
    return normalized.length >= 5 ? normalized : text.trim();
  }

  function isSsrLine(t) {
    if (!t || t.length > 400) return false;
    return SSR_LINE_RE.test(t);
  }

  function cleanText(text) {
    return text.split("\n").filter(line => {
      const t = line.trim();
      if (!t) return true;
      if (isChromeLine(t)) return false;
      if (isSsrLine(t)) return false;
      return true;
    }).join("\n").trim();
  }

  function extractTextWithMath(el) {
    if (!el) return "";
    const clone = el.cloneNode(true);
    for (const ann of Array.from(
        clone.querySelectorAll('annotation[encoding="application/x-tex"]'))) {
      const latex = (ann.textContent || "").trim();
      if (!latex) continue;
      const katexOuter = ann.closest(".katex-display, .katex") || ann.parentElement;
      if (katexOuter) {
        const isDisplay = katexOuter.classList.contains("katex-display") ||
                          (katexOuter.parentElement &&
                           katexOuter.parentElement.classList.contains("katex-display"));
        const wrapped = isDisplay ? `\n$$${latex}$$\n` : ` $${latex}$ `;
        katexOuter.replaceWith(document.createTextNode(wrapped));
      }
    }
    for (const mjx of Array.from(clone.querySelectorAll("mjx-container"))) {
      let latex = "";
      const mmlAnn = mjx.querySelector('annotation[encoding*="TeX"]');
      if (mmlAnn) latex = (mmlAnn.textContent || "").trim();
      if (!latex) latex = mjx.getAttribute("aria-label") || "";
      if (!latex) latex = mjx.getAttribute("data-latex") || "";
      if (latex) {
        const isDisplay = mjx.getAttribute("display") === "true" ||
                          mjx.getAttribute("data-display") === "block";
        const wrapped = isDisplay ? `\n$$${latex}$$\n` : ` $${latex}$ `;
        mjx.replaceWith(document.createTextNode(wrapped));
      }
    }
    for (const mathEl of Array.from(
        clone.querySelectorAll("[data-math-tex], [data-tex], [data-latex]"))) {
      const latex = mathEl.getAttribute("data-math-tex") ||
                    mathEl.getAttribute("data-tex") ||
                    mathEl.getAttribute("data-latex") || "";
      if (latex) {
        const isBlock = mathEl.tagName.toLowerCase() === "div" ||
                        mathEl.getAttribute("display") === "block";
        const wrapped = isBlock ? `\n$$${latex}$$\n` : ` $${latex}$ `;
        mathEl.replaceWith(document.createTextNode(wrapped));
      }
    }
    for (const math of Array.from(clone.querySelectorAll("math"))) {
      if (!math.isConnected) continue;
      const alttext = math.getAttribute("alttext") || "";
      if (alttext) math.replaceWith(document.createTextNode(` $${alttext}$ `));
    }
    return (clone.innerText || "").trim();
  }

  function isSSRGarbage(text) {
    if (!text || text.length < 10) return false;
    // Only flag short responses that are dominated by SSR boot markers. A long
    // response (>= 500 chars) that happens to contain "window.__oai_log" inside
    // a code block or critique is NOT garbage.
    if (text.length >= 500) return false;
    // Short response: check for multiple SSR markers OR very high JS density.
    const ssrHits = (text.match(/window\.__oai_/g) || []).length;
    if (ssrHits >= 2) return true;
    const jsRatio = (text.match(/[{}();=]/g) || []).length / text.length;
    if (jsRatio > 0.15) return true;
    return false;
  }

  // Dedicated extraction targeting only assistant-role DOM. For re_extract mode
  // we know the conversation has at least one assistant turn we want; this skips
  // the heuristic gauntlet and goes straight to it.
  function extractAssistantOnly() {
    const main = document.querySelector("main");
    if (!main) return "";
    const els = main.querySelectorAll("[data-message-author-role='assistant']");
    if (els.length === 0) return "";
    // Walk from last to first; pick the largest substantive one
    const candidates = [];
    for (let i = els.length - 1; i >= 0; i--) {
      const text = cleanText(extractTextWithMath(els[i]));
      if (text.length >= 100 && !looksLikePromptEcho(text)) {
        candidates.push({ idx: i, text, len: text.length });
      }
    }
    if (candidates.length === 0) return "";
    // Return the LAST (most recent) one
    candidates.sort((a, b) => b.idx - a.idx);
    return candidates[0].text;
  }

  // NyxID ADD: full-transcript extraction for "attach existing conversation"
  // (kind=scrape) tasks. Walks every user/assistant message node in DOM
  // (= conversation) order and returns [{role, text}], reusing the same
  // math-aware per-message extraction + chrome/SSR cleaning as the
  // single-answer path. System/tool messages are skipped.
  function extractFullTranscript() {
    const main = document.querySelector("main") || document.body;
    if (!main) return [];
    const nodes = main.querySelectorAll("[data-message-author-role]");
    const turns = [];
    for (const el of nodes) {
      const role = el.getAttribute("data-message-author-role");
      if (role !== "user" && role !== "assistant") continue;
      const text = cleanText(extractTextWithMath(el));
      if (!text) continue;
      turns.push({ role, text });
    }
    return turns;
  }

  function extractFullTranscriptSnapshot() {
    const main = document.querySelector("main") || document.body;
    if (!main) return { rendered: 0, turns: [] };
    const nodes = Array.from(main.querySelectorAll("[data-message-author-role]"));
    const turns = [];
    let fallbackIndex = 0;
    for (const el of nodes) {
      const role = el.getAttribute("data-message-author-role");
      if (role !== "user" && role !== "assistant") continue;
      const turn = el.closest('[data-testid^="conversation-turn"]');
      const testid = turn ? turn.getAttribute("data-testid") : "";
      let key = testid || `${role}#${fallbackIndex++}`;
      const text = cleanText(extractTextWithMath(el));
      if (!text) continue;
      if (!testid) key = `${key}|${text}`;
      turns.push({ key, role, text });
    }
    return { rendered: nodes.length, turns };
  }

  // NyxID ADD: wait for an existing conversation's DOM to finish rendering
  // (virtualized transcripts hydrate progressively). We are NOT waiting for
  // generation here — just for the message count to stabilize. Returns the
  // final message-node count.
  async function waitForTranscriptLoad(maxMs) {
    const start = Date.now();
    let lastCount = -1;
    let stable = 0;
    while (Date.now() - start < maxMs) {
      const count = document.querySelectorAll("[data-message-author-role]").length;
      if (count > 0 && count === lastCount) {
        stable += 1;
        if (stable >= 3) return count;
      } else {
        stable = 0;
      }
      lastCount = count;
      // Nudge virtualized transcripts to materialize earlier turns.
      try { scrollConversationToBottom("scrape-load", true); } catch {}
      await sleep(2000);
    }
    return Math.max(lastCount, 0);
  }

  function expandCollapsiblesInPage() {
    try {
      const root = document.querySelector("main") || scrollContainer() || document.body;
      if (!root) return 0;
      const maxActions = 40;
      let actions = 0;
      const expandTextRe = /Thought for|思考|Show more|显示更多|展开/i;
      const blockedButtonRe = /send|stop|model|attach|mic|voice|dictation|file|plus|new|search|发送|停止|模型|附件|添加|文件|语音|听写|搜索/i;

      function visible(el) {
        try {
          const rect = el.getBoundingClientRect();
          const style = window.getComputedStyle(el);
          return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none";
        } catch {
          return false;
        }
      }

      function inComposerArea(el) {
        try {
          if (el.closest("form, [class*='composer'], [class*='input-area'], [id='prompt-textarea']")) return true;
          const prompt = findPromptInput();
          return !!(prompt && (el === prompt || prompt.contains(el) || el.contains(prompt)));
        } catch {
          return false;
        }
      }

      function blockedButton(el) {
        const btn = el.closest("button, [role='button']");
        if (!btn) return false;
        const tid = btn.getAttribute("data-testid") || "";
        const label = btn.getAttribute("aria-label") || "";
        const text = btn.textContent || "";
        return blockedButtonRe.test(`${tid} ${label} ${text}`);
      }

      for (const details of Array.from(root.querySelectorAll("details:not([open])"))) {
        if (actions >= maxActions) break;
        if (inComposerArea(details)) continue;
        details.open = true;
        actions += 1;
      }

      for (const el of Array.from(root.querySelectorAll('[aria-expanded="false"]'))) {
        if (actions >= maxActions) break;
        if (inComposerArea(el) || blockedButton(el) || !visible(el)) continue;
        try {
          el.click();
          actions += 1;
        } catch {}
      }

      for (const el of Array.from(root.querySelectorAll("button, [role='button'], summary"))) {
        if (actions >= maxActions) break;
        if (inComposerArea(el) || blockedButton(el) || !visible(el)) continue;
        const text = (el.innerText || el.textContent || "").trim();
        if (!expandTextRe.test(text)) continue;
        try {
          el.click();
          actions += 1;
        } catch {}
      }

      if (actions > 0) log(`scrape: expanded ${actions} collapsed blocks`);
      return actions;
    } catch (e) {
      log(`scrape: expand skipped (${e.message})`);
      return 0;
    }
  }

  async function loadFullTranscriptInPage() {
    const renderStart = Date.now();
    let renderedCount = 0;
    while (Date.now() - renderStart < 20000) {
      renderedCount = document.querySelectorAll("[data-message-author-role]").length;
      if (renderedCount > 0) break;
      await sleep(700);
    }
    await sleep(1500);

    expandCollapsiblesInPage();
    await sleep(500);

    const wraps = Array.from(document.querySelectorAll('[data-testid^="conversation-turn"]')).slice(0, 2000);
    if (wraps.length > 0) {
      const turns = [];
      const seen = new Set();
      for (const w of wraps) {
        try {
          w.scrollIntoView({ block: "center" });
        } catch {}
        await sleep(150);
        const roleEl = w.querySelector("[data-message-author-role]");
        if (!roleEl) continue;
        const role = roleEl.getAttribute("data-message-author-role");
        if (role !== "user" && role !== "assistant") continue;
        const key = w.getAttribute("data-testid");
        if (!key || seen.has(key)) continue;
        const text = cleanText(extractTextWithMath(roleEl)).slice(0, 200000);
        if (!text) continue;
        seen.add(key);
        turns.push({ role, text });
      }
      renderedCount = Math.max(renderedCount, wraps.length);
      log(`scrape: rendered≈${renderedCount} turns, accumulated ${turns.length}`);
      return turns;
    } else {
      let lastHeight = -1;
      let stableHeight = 0;
      for (let i = 0; i < 50; i++) {
        try {
          const sc = scrollContainer();
          sc.scrollTop = 0;
        } catch {}
        await sleep(700);
        let height = 0;
        try {
          height = scrollContainer().scrollHeight || 0;
        } catch {}
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
      let bottomStable = 0;
      for (let i = 0; i < 120 && acc.size < 2000; i++) {
        const snapshot = extractFullTranscriptSnapshot();
        renderedCount = Math.max(renderedCount, snapshot.rendered || 0);
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
          const sc = scrollContainer();
          const step = Math.floor((sc.clientHeight || window.innerHeight || 800) * 0.8);
          sc.scrollTop = Math.min(sc.scrollHeight, sc.scrollTop + step);
        } catch {}
        await sleep(600);
        let atBottom = false;
        try {
          const sc = scrollContainer();
          atBottom = sc.scrollTop + sc.clientHeight >= sc.scrollHeight - 4;
        } catch {}
        if (atBottom) {
          bottomStable += 1;
          if (bottomStable >= 2) break;
        } else {
          bottomStable = 0;
        }
      }
      log(`scrape: rendered≈${renderedCount} turns, accumulated ${acc.size}`);
      return order.map(key => acc.get(key)).filter(Boolean);
    }
  }

  function extractResponseText() {
    const main = document.querySelector("main");
    if (!main) return "";
    const fullText = extractTextWithMath(main);

    if (sentPromptText.length > 500) {
      const tailAnchor = sentPromptText.slice(-100).trim();
      let idx = fullText.lastIndexOf(tailAnchor);
      if (idx < 0 && tailAnchor.length > 50) idx = fullText.lastIndexOf(tailAnchor.slice(-50));
      if (idx >= 0) {
        const after = cleanText(fullText.slice(idx + tailAnchor.length));
        if (after.length > 100) return after;
      }
    }

    const novelText = postSendNovelText(fullText);
    if (novelText.length > 100 && !looksLikePromptEcho(novelText)) return novelText;

    const candidates = [];
    const allBlocks = main.querySelectorAll("div, article, section");
    for (const el of allBlocks) {
      const text = extractTextWithMath(el);
      if (text.length < 200) continue;
      candidates.push({ el, text, len: text.length });
    }
    candidates.sort((a, b) => b.len - a.len);
    for (const cand of candidates) {
      const cleaned = cleanText(cand.text);
      if (cleaned.length < 200) continue;
      const pageLen = fullText.length;
      if (cleaned.length > pageLen * 0.95 && candidates.length > 3) continue;
      if (looksLikePromptEcho(cleaned)) continue;
      if (!hasPostSendNovelContent(cleaned)) continue;
      if (sentPromptText.length > 500) {
        const promptStart = sentPromptText.slice(0, 200).trim();
        if (cleaned.startsWith(promptStart)
            && cleaned.length <= sentPromptText.length * 1.1) continue;
      }
      return cleaned;
    }

    const s0Selectors = [
      "[data-message-author-role='assistant']",
      "[data-testid*='conversation-turn']",
      "article",
      "div[class*='markdown']",
      "div[class*='prose']",
      "div.markdown",
      "[class*='agent-turn']",
    ];
    for (const sel of s0Selectors) {
      try {
        const els = document.querySelectorAll(sel);
        if (els.length === 0) continue;
        for (let i = els.length - 1; i >= 0; i--) {
          const text = extractTextWithMath(els[i]);
          const cleaned = cleanText(text);
          if (cleaned.length < 200) continue;
          if (looksLikePromptEcho(cleaned)) continue;
          if (!hasPostSendNovelContent(cleaned)) continue;
          if (sentPromptText.length > 30) {
            const ps = sentPromptText.slice(0, 40).trim();
            if (cleaned.startsWith(ps) && cleaned.length < sentPromptText.length * 1.2) continue;
          }
          return cleaned;
        }
      } catch {}
    }

    if (fullText.length < 100) return "";
    if (sentPromptText.length > 30) {
      for (const anchorLen of [80, 50, 30]) {
        const anchor = sentPromptText.slice(0, anchorLen).trim();
        const idx = fullText.indexOf(anchor);
        if (idx >= 0) {
          let endIdx = idx + sentPromptText.length;
          if (sentPromptText.length > 60) {
            const tail = sentPromptText.slice(-40).trim();
            const tailIdx = fullText.indexOf(tail, idx);
            if (tailIdx >= 0) endIdx = Math.max(endIdx, tailIdx + tail.length);
          }
          const after = cleanText(fullText.slice(endIdx));
          if (after.length > 100) return after;
        }
      }
    }

    return "";
  }

  function isStillGenerating() {
    const domSignal = !!(
      document.querySelector("button[aria-label='Stop generating']") ||
      document.querySelector("button[aria-label='Stop streaming']") ||
      document.querySelector("button[aria-label='停止生成']") ||
      document.querySelector("button[aria-label='停止流式传输']") ||
      document.querySelector("button[data-testid='stop-button']") ||
      document.querySelector("[class*='result-streaming']") ||
      document.querySelector("[class*='streaming']") ||
      document.querySelector("[class*='thinking']") ||
      document.querySelector("[class*='reasoning']") ||
      document.querySelector("[class*='progress']")
    );
    if (domSignal) return true;
    // Text-layer probe for ChatGPT Pro reasoning indicators that don't expose
    // stable class hooks. The page text contains these literals while the Pro
    // reasoner is still thinking and before the visible answer streams in.
    // Without this fallback the userscript trips on "Pro thinking" pages that
    // look stable but are mid-generation.
    try {
      const main = document.querySelector("main");
      if (!main) return false;
      const txt = main.innerText || "";
      // "Pro thinking" — Pro reasoning state preamble
      // "Extended Pro" — appears in reasoner footer DURING reasoning, not after
      // "Thought for" — only appears AFTER reasoning completes (post-think)
      // We treat the page as still generating if the reasoning preamble is
      // present AND the post-think marker "Thought for" is NOT yet visible.
      const proPreamble = /Pro thinking|Extended Pro|Reasoning…/i.test(txt);
      const postThink = /Thought for\s+\d+(?:\s*m\s*\d+\s*s|\s*s|\s+min)/i.test(txt);
      if (proPreamble && !postThink) return true;
    } catch {}
    return false;
  }

  async function waitForResponse(task_id, noOutputIdleTimeout = NO_OUTPUT_IDLE_TIMEOUT) {
    log(`Waiting for ChatGPT response (pre-send assistant count was ${preSubmitAssistantCount})...`);
    const startTime = Date.now();
    let lastResponseText = "";
    let lastStableKey = "";
    let stableCount = 0;
    let lastLogTime = 0;
    let lastHeartbeat = 0;
    let countIncreasedLogged = false;
    while (Date.now() - startTime < MAX_WAIT) {
      await sleep(STABLE_INTERVAL);
      scrollConversationToBottom("wait");
      await sleep(500);
      // Require strict count increase before trusting any extractResponseText
      // output. Without this, the multi-strategy fallback can return prior-turn
      // text that happens to be "stable" because no new generation has rendered.
      const curCount = newAssistantCount();
      let responseText = (curCount > preSubmitAssistantCount)
        ? extractAssistantOnly()    // count increased: take the LAST assistant message only
        : "";                       // count not yet increased: don't even consider stability
      if (curCount > preSubmitAssistantCount && responseText.length < 5) {
        scrollConversationToBottom("empty-after-count-increase", true);
        await sleep(1000);
        responseText = extractAssistantOnly();
      }
      if (curCount > preSubmitAssistantCount && !countIncreasedLogged) {
        log(`new assistant message detected (count ${preSubmitAssistantCount} → ${curCount})`);
        countIncreasedLogged = true;
      }
      const generating = isStillGenerating();
      const elapsed = Math.floor((Date.now() - startTime) / 1000);
      const mainLen = (document.querySelector("main")?.innerText || "").length;
      const nowMs = Date.now();
      if (task_id && nowMs - lastHeartbeat >= 60000) {
        lastHeartbeat = nowMs;
        let ack = null;
        try {
          // NyxID /ack: worker + phase + short human phase_detail (bedc's
          // structured metrics object is collapsed into the phase_detail string).
          ack = await serverPost("/ack", {
            task_id,
            worker: workerLabel(),
            phase: generating ? "generating" : "waiting",
            phase_detail:
              `elapsed=${elapsed}s extracted=${responseText.length} ` +
              `page=${mainLen} stable=${stableCount} gen=${generating} ` +
              `url=${window.location.href.slice(-40)}`,
          });
        } catch {}
        if (ack && ack.status === "cancelled") {
          throw new Error(`Task cancelled by server: ${task_id}`);
        }
      }
      if (elapsed - lastLogTime >= 300) {
        lastLogTime = elapsed;
        log(`Wait: ${elapsed}s, extracted=${responseText.length}, page=${mainLen}, stable=${stableCount}, gen=${generating}, url=${window.location.href.slice(-30)}`);
      }
      if (
        !generating &&
        responseText.length < 5 &&
        Date.now() - startTime >= noOutputIdleTimeout
      ) {
        throw new Error(
          `No assistant output after ${Math.floor(noOutputIdleTimeout / 1000)}s ` +
          `(page=${mainLen}, url=${window.location.href.slice(-60)})`
        );
      }
      if (responseText.length >= 5) {
        if (looksLikePromptEcho(responseText)) {
          if (stableCount === 0) log(`Prompt echo detected (${responseText.length} chars) — waiting`);
          stableCount = 0; lastResponseText = ""; lastStableKey = ""; continue;
        }
        if (isSSRGarbage(responseText)) {
          if (stableCount === 0) log(`SSR garbage detected — page hydrating, waiting`);
          stableCount = 0; lastResponseText = ""; lastStableKey = ""; continue;
        }
        const stableKey = stableResponseKey(responseText);
        if (stableKey === lastStableKey) {
          stableCount++;
          lastResponseText = responseText;
          let minChecks;
          if (responseText.length >= 2000) minChecks = STABLE_CHECKS;
          else if (responseText.length >= 200) minChecks = STABLE_CHECKS + 2;
          else minChecks = STABLE_CHECKS * 3;
          const stableEnough = stableCount >= minChecks && !generating;
          const stableOverride = stableCount >= minChecks + 3;
          if (stableEnough || stableOverride) {
            log(`Response complete: ${responseText.length} chars (stable ${stableCount * STABLE_INTERVAL / 1000}s, gen=${generating})`);
            return responseText;
          }
        } else {
          stableCount = 0;
          lastResponseText = responseText;
          lastStableKey = stableKey;
        }
      } else if (generating) {
        stableCount = 0;
      }
    }
    log(`TIMEOUT (${MAX_WAIT/1000}s), returning partial: ${lastResponseText.length} chars`);
    return lastResponseText;
  }

  // ── Process a task (multi-turn navigation + reload-safe) ─────────────
  // Internal task shape (built from the NyxID /task status:"task" response):
  //   { task_id, prompt, conversation_url, is_followup, conversation_id,
  //     re_extract, pdf_base64, pdf_name, tag, model }
  // NOTE: the NyxID server has no re_extract mode in its wire spec, so
  // task.re_extract is always falsy here and the RE-EXTRACT branch is inert.
  // It is preserved verbatim so the DOM-resume logic stays identical to BEDC.
  // NyxID ADD: scrape an existing conversation by URL (kind=scrape). Navigate
  // to the target /c/<uuid> (the RAW url — an attached conversation may live
  // outside the pool's project, so we do NOT pin to project here), wait for the
  // transcript DOM to settle, extract every user/assistant turn, and POST it to
  // /worker/transcript. No prompt is injected and nothing is sent. ChatGPT
  // full-reloads on the first /c/<uuid> navigation, so we save state and resume
  // after reload via the same nav machinery the prompt flow uses.
  async function processScrapeTask(task) {
    const { task_id, conversation_url } = task;
    log(`=== Task: ${task_id} [SCRAPE] ${(conversation_url || "").slice(-50)} ===`);
    busy = true;
    updatePanel();
    try {
      const target = conversation_url;
      if (!target) throw new Error("scrape task has no conversation_url");
      // Defense-in-depth: the server validates attach URLs to ChatGPT origins,
      // but assert it again before assigning to location.href so untrusted
      // task input can never redirect this logged-in tab off-origin (only
      // matters if the pool server itself is compromised — cheap to close).
      if (!/^https:\/\/(chatgpt\.com|chat\.openai\.com)\//.test(target)) {
        throw new Error("scrape target is not a ChatGPT URL");
      }
      const idMatch = target.match(/\/c\/([a-f0-9-]{6,})/);
      const onTarget = window.location.href === target
        || (idMatch && window.location.href.includes(idMatch[1]));
      if (!onTarget) {
        tabSet("navigating", true);
        tabSet("nav_task_id", task_id);
        saveTaskState(task);
        setTaskPhase("navigating");
        setInFlightTaskId(task_id);
        log(`Navigating to ${target.slice(-60)} for scrape ...`);
        busy = false;
        updatePanel();
        window.location.href = target;
        return;
      }
      try { await serverPost("/ack", { task_id, worker: workerLabel(), phase: "scraping" }); } catch {}
      await sleep(2500); // let the SPA mount the conversation
      const turns = await loadFullTranscriptInPage();
      log(`scrape: ${turns.length} turns loaded`);
      const chatUrl = currentChatUrl();
      const res = await serverPost("/worker/transcript", {
        task_id, worker: workerLabel(), turns, chatgpt_url: chatUrl,
      });
      log(`DONE (scrape): ${task_id} — ${turns.length} turns, imported ${res && res.imported_pairs != null ? res.imported_pairs : "?"} pairs`);
      clearTaskState();
      setInFlightTaskId("");
    } catch (err) {
      log(`ERROR (scrape): ${err.message}`);
      // Post an empty transcript so the control task completes instead of
      // looping on lease expiry; the consumer sees an empty imported session.
      try {
        await serverPost("/worker/transcript", {
          task_id, worker: workerLabel(), turns: [], chatgpt_url: currentChatUrl(),
        });
      } catch {}
      clearTaskState();
      setInFlightTaskId("");
    } finally {
      busy = false;
      updatePanel();
    }
  }

  async function processTask(task) {
    const { task_id, prompt, conversation_url, is_followup, conversation_id, re_extract, pdf_base64, pdf_name, tag } = task;
    const noOutputIdleTimeout = (tag === "bedc-deep-board-refill")
      ? REFILL_NO_OUTPUT_IDLE_TIMEOUT
      : NO_OUTPUT_IDLE_TIMEOUT;
    busy = true;
    updatePanel();

    // NyxID ADD: scrape tasks attach an existing conversation by URL. They run
    // before project enforcement (the target may be outside the pool project)
    // and never inject a prompt.
    if (task.kind === "scrape") {
      await processScrapeTask(task);
      return;
    }

    if (!isInsideRequiredProject()) {
      navigateTaskBackToProject(task, "outside project before task");
      return;
    }

    // re_extract mode: server says "this conversation already has the response
    // we want — just navigate there and extract the latest assistant message,
    // do not enter or send anything". (Inert under NyxID; preserved for parity.)
    if (re_extract) {
      log(`=== Task: ${task_id} [RE-EXTRACT] conv=${(conversation_id || "").slice(0, 12)} ===`);
      try {
        // Re-pin server-provided URL into the project namespace in case it was
        // stored as a bare /c/<id> from a drifted session.
        const pinnedConv = pinToProject(conversation_url);
        if (pinnedConv && !window.location.href.startsWith(pinnedConv)) {
          tabSet("navigating", true);
          tabSet("nav_task_id", task_id);
          saveTaskState(task);
          setTaskPhase("navigating");
          log(`Navigating to ${pinnedConv.slice(-60)} for re-extract ...`);
          busy = false;
          updatePanel();
          window.location.href = pinnedConv;
          return;
        }
        try { await serverPost("/ack", { task_id, worker: workerLabel(), phase: "re-extract" }); } catch {}
        await sleep(3000); // settle DOM
        scrollConversationToBottom("re-extract", true);
        await sleep(1000);
        if (prompt) setSentPrompt(prompt);
        // Try dedicated assistant-only extraction first (bypasses heuristics
        // that can fall to prompt echo when sentPromptText isn't perfectly
        // aligned). Fall back to extractResponseText if none found.
        let response = extractAssistantOnly();
        if (!response || response.length < 100) {
          log(`re-extract: assistant-only got ${response?.length || 0} chars; falling back to extractResponseText`);
          response = extractResponseText();
        } else {
          log(`re-extract: assistant-only got ${response.length} chars`);
        }
        if (!response || response.length < 100) {
          throw new Error(`re-extract: nothing meaningful (${response?.length || 0} chars)`);
        }
        const chatUrl = currentChatUrl();
        await serverPost("/result", {
          task_id, response, chatgpt_url: chatUrl,
          worker: workerLabel(), model: task.model || "unknown",
        });
        log(`DONE (re-extract): ${task_id} (${response.length} chars)`);
        clearTaskState();
        setInFlightTaskId("");
      } catch (err) {
        log(`ERROR (re-extract): ${err.message}`);
        try {
          await serverPost("/result", {
            task_id, response: `ERROR (re-extract): ${err.message}`,
            chatgpt_url: currentChatUrl(), worker: workerLabel(),
            model: task.model || "unknown",
          });
        } catch {}
        clearTaskState();
        setInFlightTaskId("");
      } finally {
        busy = false;
        updatePanel();
      }
      return;
    }

    // Re-entry guard. If this same task_id is in flight and we are currently on
    // a /c/<uuid> page (= ChatGPT already accepted our first prompt and started
    // generating), DO NOT re-enter the prompt. Just resume waitForResponse.
    // ChatGPT triggers a full page reload when the URL first changes from
    // chatgpt.com/ to chatgpt.com/c/<uuid>, which loses our in-memory state but
    // the in-flight task survives. The guard applies to follow-up tasks too:
    // long Pro reasoning turns (>30 min) inside an existing /c/<uuid> page can
    // trigger DOM remount / focus reset, which re-enters processTask. Without
    // including follow-ups, the same prompt gets submitted twice and the polite
    // restatement overwrites the real long response captured by the extractor.
    const onConvPage = /\/c\/[a-f0-9-]{6,}/.test(window.location.href);
    if (getInFlightTaskId() === task_id && onConvPage) {
      log(`=== Task: ${task_id} [RESUMING on existing chat ${currentChatUrl().slice(-40)}] ===`);
      try { await serverPost("/ack", { task_id, worker: workerLabel(), phase: "resuming" }); } catch {}
      setTaskPhase("processing");
      setSentPrompt(prompt);
      scrollConversationToBottom("resume", true);
      await sleep(500);
      capturePostSendState();
      try {
        const response = await waitForResponse(task_id, noOutputIdleTimeout);
        if (!response || response.length < 5) {
          throw new Error(`Resumed wait got no response (${response?.length || 0} chars)`);
        }
        const chatUrl = currentChatUrl();
        log(`Resumed chat URL captured: ${chatUrl.slice(-50) || "(none)"}`);
        await serverPost("/result", {
          task_id, response, chatgpt_url: chatUrl,
          worker: workerLabel(), model: task.model || "unknown",
        });
        log(`DONE (resumed): ${task_id} (${response.length} chars)`);
        clearTaskState();
        setInFlightTaskId("");
      } catch (err) {
        log(`ERROR (resumed): ${err.message}`);
        try {
          await serverPost("/result", {
            task_id, response: `ERROR (resumed): ${err.message}`,
            chatgpt_url: currentChatUrl(), worker: workerLabel(),
            model: task.model || "unknown",
          });
        } catch {}
        clearTaskState();
        setInFlightTaskId("");
      } finally {
        busy = false;
        updatePanel();
      }
      return;
    }

    log(`=== Task: ${task_id} ${is_followup ? "[FOLLOW-UP]" : "[NEW]"} conv=${(conversation_id || "").slice(0, 12)} ===`);

    try {
      // Navigation logic — three cases:
      // (a) follow-up + conversation_url provided + we are NOT on it → navigate there
      // (b) new task + we're not on a fresh chat page → navigate to fresh chat
      // (c) otherwise stay where we are
      //
      // pinToProject pins any server-provided conversation_url into the required
      // project namespace before deciding to navigate (no-op when no project is
      // required). pinToProject is idempotent so already-pinned URLs are unchanged.
      const targetUrl = (is_followup && conversation_url) ? pinToProject(conversation_url) : null;
      const needNavToConv = targetUrl && !window.location.href.startsWith(targetUrl);
      const needNavToFresh = !targetUrl && !isOnNewChatPage();

      if (needNavToFresh) {
        // Prefer in-page "New Chat" button click (SPA, no redirect), fall back
        // to URL navigation only if no button found.
        log(`Need fresh chat. Current URL: ${window.location.href.slice(-60)}`);
        const ok = await clickNewChatButton();
        if (ok) {
          log(`Clicked New Chat button; on fresh chat now.`);
          // No reload — keep going in same script instance
        } else {
          log(`No New Chat button found; falling back to URL navigation`);
          tabSet("navigating", true);
          tabSet("nav_task_id", task_id);
          saveTaskState(task);
          setTaskPhase("navigating");
          busy = false;
          updatePanel();
          // If a project is required and we're inside a ChatGPT Project (URL
          // like /g/g-p-XXXXXX-name/c/<uuid>), fall back to the project's root
          // URL so we DON'T leave the Project (which would lose the
          // project-attached PDF and any project-wide instructions). With no
          // project required, fall back to chatgpt.com root with the tab's
          // nyx= flag pinned.
          const flag = workerLabelFlag();
          let fallbackUrl;
          if (requiredProjectUrl) {
            const m = window.location.pathname.match(/^(\/g\/g-p-[a-zA-Z0-9_-]+)/);
            fallbackUrl = m
              ? `https://chatgpt.com${m[1]}/project?nyx=${encodeURIComponent(flag)}`
              : (projectEntryUrl() || `https://chatgpt.com/?nyx=${encodeURIComponent(flag)}`);
          } else {
            fallbackUrl = `https://chatgpt.com/?nyx=${encodeURIComponent(flag)}`;
          }
          log(`fallback URL: ${fallbackUrl} (worker=${workerLabel()})`);
          window.location.href = fallbackUrl;
          return;
        }
      } else if (needNavToConv) {
        tabSet("navigating", true);
        tabSet("nav_task_id", task_id);
        saveTaskState(task);
        setTaskPhase("navigating");
        log(`Navigating to existing conv ${(targetUrl || "").slice(-60)} ...`);
        busy = false;
        updatePanel();
        window.location.href = targetUrl;
        return;
      }

      if (!isInsideRequiredProject()) {
        navigateTaskBackToProject(task, "navigation left project");
        return;
      }

      // ACK (page_ready)
      try { await serverPost("/ack", { task_id, worker: workerLabel(), phase: "page_ready" }); } catch {}
      setTaskPhase("processing");
      // Mark this task in-flight BEFORE we send. Also save the task body so a
      // full reload mid-flight can read prompt back without hitting the server
      // queue (which would trigger needNavToFresh again).
      setInFlightTaskId(task_id);
      saveTaskState(task);

      // Wait for prompt input. 90s — ChatGPT fresh chat can be slow to hydrate,
      // especially after a New Chat button click that unmounts and remounts the
      // composer. Log every 15s for visibility.
      let retries = 0;
      while (!findPromptInput() && retries < 90) {
        await sleep(1000);
        retries++;
        if (retries > 0 && retries % 15 === 0) {
          log(`Still waiting for prompt input... ${retries}s, url=${window.location.href.slice(-50)}`);
        }
      }
      if (!findPromptInput()) {
        throw new Error(`Prompt input not found after 90s (url=${window.location.href})`);
      }
      log(`Page ready (${is_followup ? "existing conv" : "fresh chat"}) after ${retries}s`);

      // PDF attach BEFORE prompt entry, only on first turn of a fresh
      // conversation (non-followup) AND only if server provided pdf_base64.
      // Follow-up turns inherit the PDF from earlier turns via conversation
      // memory, so re-uploading is wasted work.
      if (!is_followup && pdf_base64) {
        try {
          const ok = await uploadPDF(pdf_base64, pdf_name || "main.pdf");
          if (!ok) log("PDF upload failed — proceeding without PDF context");
        } catch (e) {
          log(`PDF upload exception: ${e.message} — proceeding without PDF`);
        }
      }

      // Enter prompt
      const entered = await enterPrompt(prompt);
      if (!entered) throw new Error("Failed to enter prompt text");
      setSentPrompt(prompt);

      // Wait for send button
      log("Waiting for send button to enable...");
      let sendReady = false;
      for (let i = 0; i < 30; i++) {
        const btn = findSendButton();
        if (btn && !btn.disabled) { sendReady = true; log(`Send ready after ${i}s`); break; }
        await sleep(1000);
      }
      if (!sendReady) log("WARN: send still disabled after 30s, will force-click");

      const urlBefore = window.location.href;
      const sent = await clickSend();
      if (!sent) throw new Error("Failed to click send");

      // For new chat: wait for URL change to /c/<id>
      // For follow-up: URL should NOT change (we stay in same /c/<id>)
      log(`Sent. urlBefore=${urlBefore.slice(-40)}`);
      if (!is_followup) {
        let urlChanged = false;
        for (let i = 0; i < 60; i++) {
          await sleep(1000);
          if (window.location.href !== urlBefore) {
            urlChanged = true;
            log(`URL changed to: ${window.location.href.slice(-40)}`);
            // Conversation URL is now known — pin it server-side so future
            // turns can be routed back to this chat.
            try {
              await serverPost("/pin-conv-url", {
                task_id, worker: workerLabel(), chatgpt_url: currentChatUrl(),
              });
            } catch {}
            break;
          }
          if (isStillGenerating()) { log("Generation detected (same page)"); break; }
        }
        if (!urlChanged && !isStillGenerating()) {
          log("WARN: URL did not change and no generation after 60s");
        }
      } else {
        // For follow-up: just wait for generation to start
        for (let i = 0; i < 30; i++) {
          await sleep(1000);
          if (isStillGenerating()) { log("Follow-up generation started"); break; }
        }
      }

      await sleep(5000); // settle DOM
      scrollConversationToBottom("post-send", true);
      await sleep(500);
      capturePostSendState();
      const response = await waitForResponse(task_id, noOutputIdleTimeout);

      if (!response || response.length < 5) {
        throw new Error(`Response too short or empty (${response?.length || 0} chars)`);
      }

      // Capture chat URL for the server to pin to conversation_id
      const chatUrl = currentChatUrl();
      log(`Chat URL captured: ${chatUrl.slice(-50) || "(none)"}`);

      await serverPost("/result", {
        task_id,
        response,
        chatgpt_url: chatUrl,
        worker: workerLabel(),
        model: task.model || "unknown",
      });
      log(`DONE: ${task_id} (${response.length} chars)`);
      clearTaskState();
      setInFlightTaskId("");
    } catch (err) {
      log(`ERROR: ${err.message}`);
      try {
        await serverPost("/result", {
          task_id, response: `ERROR: ${err.message}`,
          chatgpt_url: currentChatUrl(),
          worker: workerLabel(), model: task.model || "unknown",
        });
      } catch {}
      clearTaskState();
      setInFlightTaskId("");
    } finally {
      busy = false;
      updatePanel();
    }
  }

  // ── Identity (PER-TAB, not per-script) ───────────────────────────────
  // GM_setValue is shared across all tabs that have this userscript installed,
  // so persisting the worker label there causes multiple tabs to share an
  // identity and the server dispatches the same task to all of them
  // concurrently. Use sessionStorage (per-tab) instead, fall back to
  // window.name + URL flag.
  //
  // workerLabel() is PINNED on first call and reused for the lifetime of this
  // tab. The ?nyx=N URL flag is authoritative when present: ?nyx=2 → tab_2.
  // After ChatGPT redirects /?nyx=N → /c/<uuid> the URL flag is gone, but the
  // sessionStorage value we just wrote keeps the tab pinned — so a tab's
  // identity stays stable for its full session (and the per-tab GM namespace
  // via tabSet/tabGet stays stable too).
  function labelFromFlag(flag) {
    const f = String(flag || "").trim();
    if (!f) return "";
    // Bare number → tab_N; otherwise use the raw label as given.
    return /^[0-9]+$/.test(f) ? `tab_${f}` : f;
  }
  function workerLabel() {
    try {
      const m = window.location.search.match(/[?&]nyx=([^&]+)/);
      if (m) {
        const id = labelFromFlag(decodeURIComponent(m[1]));
        if (id) {
          sessionStorage.setItem("nyxid_worker_label", id);
          return id;
        }
      }
      const stored = sessionStorage.getItem("nyxid_worker_label");
      if (stored) return stored;
      const dflt = defaultWorkerLabel();
      sessionStorage.setItem("nyxid_worker_label", dflt);
      return dflt;
    } catch {
      // Private mode / sessionStorage disabled — fall back to window.name
      if (!window.name || !window.name.startsWith("tab_")) {
        const m = window.location.search.match(/[?&]nyx=([^&]+)/);
        window.name = m ? labelFromFlag(decodeURIComponent(m[1])) : defaultWorkerLabel();
      }
      return window.name;
    }
  }

  // The numeric/raw flag that reconstructs this worker's ?nyx= value for
  // navigate-back URLs. tab_2 → "2"; a custom label → the label itself.
  function workerLabelFlag() {
    const lbl = workerLabel();
    const m = lbl.match(/^tab_(.+)$/);
    return m ? m[1] : lbl;
  }

  // Per-tab namespace for GM_setValue / GM_getValue. GM storage is shared
  // across ALL tabs running the userscript, so two tabs writing
  // `current_task` simultaneously will trample each other. Scoping every key by
  // workerLabel() gives each tab its own private namespace.
  function tabSet(k, v) { return GM_setValue(`${workerLabel()}_${k}`, v); }
  function tabGet(k, d) { return GM_getValue(`${workerLabel()}_${k}`, d); }

  function navigateTaskBackToProject(task, reason) {
    // No-op when no project is required.
    if (!requiredProjectUrl) return;
    const taskId = (task && task.task_id) || "";
    const target = /\/c\/[a-f0-9-]{6,}/.test(window.location.href)
      ? pinToProject(window.location.href)
      : projectEntryUrl();
    if (!target) return;
    tabSet("navigating", true);
    tabSet("nav_task_id", taskId);
    if (task) saveTaskState(task);
    setTaskPhase("navigating");
    log(`${reason}; navigating to project ${target.slice(-80)}`);
    busy = false;
    updatePanel();
    window.location.href = target;
  }

  // ── Main loop ────────────────────────────────────────────────────────
  function _readActive() {
    try {
      return sessionStorage.getItem("nyxid_active") === "1" && isInsideRequiredProject();
    } catch { return false; }
  }

  // If a required project is set and we're not inside it: navigate back (when
  // this tab opted in via ?nyx=) or pause. No required project ⇒ always false
  // (never blocks polling).
  function enforceProjectBeforePolling() {
    if (!requiredProjectUrl) return false;
    if (isInsideRequiredProject()) return false;
    const target = projectEntryUrl();
    if (window.location.search.includes("nyx=") && target) {
      log(`Project required; navigating to ${target}`);
      window.location.href = target;
      return true;
    }
    try { sessionStorage.setItem("nyxid_active", "0"); } catch {}
    active = false;
    log("Outside required project; polling paused");
    updatePanel();
    return true;
  }

  // Build the internal task object from the NyxID /task status:"task" response,
  // whose fields are siblings of `status`. Maps onto the shape processTask
  // expects (task_id, prompt, conversation_url, is_followup, conversation_id,
  // model, tag, pdf_base64, pdf_name).
  function buildTaskFromResponse(resp) {
    return {
      task_id: resp.task_id,
      prompt: resp.prompt,
      conversation_id: resp.conversation_id || "",
      conversation_url: resp.conversation_url || "",
      is_followup: !!resp.is_followup,
      model: resp.model || "unknown",
      tag: resp.tag || "",
      pdf_base64: resp.pdf_base64 || "",
      pdf_name: resp.pdf_name || "",
      // NyxID ADD: "scrape" tasks attach an existing conversation by URL —
      // navigate there, extract the whole transcript, post it back. Default
      // "prompt" preserves the normal inject-and-answer flow.
      kind: resp.kind || "prompt",
      // No re_extract in the NyxID wire spec; default false (branch stays inert).
      re_extract: !!resp.re_extract,
    };
  }

  async function pollLoop() {
    while (true) {
      // Gate on configured (base_url + token) AND active.
      if (!isConfigured()) {
        await sleep(POLL_INTERVAL);
        continue;
      }
      active = _readActive();
      if (active && !busy) {
        if (enforceProjectBeforePolling()) {
          await sleep(POLL_INTERVAL);
          continue;
        }
        try {
          const resp = await serverGet(
            `/task?worker=${encodeURIComponent(workerLabel())}`
          );
          // Update cached required_project_url from EVERY response (idle or task).
          if (resp && Object.prototype.hasOwnProperty.call(resp, "required_project_url")) {
            setRequiredProjectUrl(resp.required_project_url || "");
          }
          if (resp && resp.status === "task" && resp.task_id) {
            if (!_readActive()) {
              log("Task available but PAUSED — skipping");
            } else {
              // If the server handed us the SAME task_id we already had
              // in-flight (idempotent re-claim), processTask's resume guard
              // picks up where we left off rather than restarting.
              const task = buildTaskFromResponse(resp);
              await processTask(task);
            }
          }
        } catch (err) {
          if (logHistory.length === 0 || !logHistory[logHistory.length-1].includes("unreachable")) {
            log(`Server unreachable (${baseUrl() || "not configured"})`);
          }
        }
      }
      await sleep(POLL_INTERVAL);
    }
  }

  // ── Bootstrap ────────────────────────────────────────────────────────
  async function init() {
    log(`NyxID Oracle Worker ${SCRIPT_VERSION} loaded — ${isConfigured() ? (active ? "ACTIVE" : "PAUSED") : "NOT CONFIGURED"} — worker=${workerLabel()}`);
    if (!isConfigured()) {
      log("Configure base URL + worker token in the panel (⚙ Settings) to begin");
    }

    const phase = getTaskPhase();
    const navTaskId = tabGet("nav_task_id", "");
    const navFlag = tabGet("navigating", false);
    const urlHasFlag = window.location.search.includes("nyx=");
    const inFlightId = getInFlightTaskId();
    const inFlightAgeMin = Math.floor(getInFlightAgeMs() / 60000);
    const storedActive = (() => {
      try { return sessionStorage.getItem("nyxid_active") === "1"; }
      catch { return false; }
    })();

    // Project enforcement at boot only applies when a project is cached AND this
    // tab opted in via ?nyx=. requiredProjectUrl starts empty each load and is
    // re-learned on the first poll, so we don't block boot on it here.
    if (urlHasFlag && requiredProjectUrl && !isInsideRequiredProject()) {
      const target = projectEntryUrl();
      if (target) {
        log(`Project required; navigating to ${target}`);
        window.location.href = target;
        return;
      }
    }

    if ((inFlightId || storedActive) && ensureInProject()) return;

    // If we have an in-flight task that's clearly stuck (>3h), give up — clear
    // flags; pollLoop will get the next task.
    if (inFlightId && inFlightAgeMin > 180) {
      log(`Stale in-flight ${inFlightId} (${inFlightAgeMin}m old) — clearing`);
      setInFlightTaskId("");
      clearTaskState();
    }

    // Full-page-reload landing on /c/<uuid> with an in-flight task means
    // ChatGPT redirected us mid-task. The next pollLoop cycle will receive the
    // same task from the server (idempotent re-claim) and processTask's
    // RESUMING branch will take over. Self-report the URL so the server can pin
    // it to the conversation_id for future routing.
    if (inFlightId && /\/c\/[a-f0-9-]/.test(window.location.href) && isConfigured()) {
      log(`Detected mid-task reload on /c/<uuid>; in-flight=${inFlightId} (${inFlightAgeMin}m). pollLoop will resume.`);
      try {
        await serverPost("/pin-conv-url", {
          task_id: inFlightId,
          chatgpt_url: currentChatUrl(),
          worker: workerLabel(),
        });
      } catch {}
    }

    if (phase === "navigating" && navTaskId && (navFlag || urlHasFlag)) {
      log(`Resuming after navigation for task: ${navTaskId}`);
      tabSet("nav_task_id", "");
      tabSet("navigating", false);
      const savedTask = loadTaskState();
      clearTaskState();

      if (urlHasFlag) {
        const cleanUrl = window.location.href.replace(/[?&]nyx=[^&]+/, "").replace(/\?$/, "");
        history.replaceState(null, "", cleanUrl);
      }

      await sleep(3000);

      // Prefer the saved task (preserves is_followup + conversation context).
      // Fall back to re-fetching if state was lost.
      let task = savedTask;
      if ((!task || !task.task_id) && isConfigured()) {
        try {
          const resp = await serverGet(`/task?worker=${encodeURIComponent(workerLabel())}`);
          if (resp && Object.prototype.hasOwnProperty.call(resp, "required_project_url")) {
            setRequiredProjectUrl(resp.required_project_url || "");
          }
          if (resp && resp.status === "task" && resp.task_id) {
            task = buildTaskFromResponse(resp);
          }
        } catch (e) { log(`Re-fetch failed: ${e.message}`); }
      }
      if (task && task.task_id) {
        log(`Resumed task: ${task.task_id} prompt=${task.prompt?.length || 0} chars followup=${!!task.is_followup}`);
        await processTask(task);
      } else {
        log("WARN: no task to resume after navigation");
      }
    } else if (phase === "navigating") {
      log("Clearing stale navigation state (user browsing, not nyxid)");
      tabSet("nav_task_id", "");
      tabSet("navigating", false);
      clearTaskState();
    }

    pollLoop();
  }

  if (document.readyState === "complete") setTimeout(init, 2000);
  else window.addEventListener("load", () => setTimeout(init, 2000));
})();
