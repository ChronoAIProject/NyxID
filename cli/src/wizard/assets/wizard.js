(() => {
  "use strict";

  // ---- bootstrap ----

  const CSRF = document.querySelector('meta[name="wizard-csrf"]')?.getAttribute("content") || "";
  const FLOW = document.querySelector('meta[name="wizard-flow"]')?.getAttribute("content") || "ai-key";
  const BASE_URL = (document.querySelector('meta[name="wizard-base-url"]')?.getAttribute("content") || "").replace(/\/+$/, "");
  let postInFlight = false;   // swallow beforeunload cancel while a POST is open

  const originEl = document.getElementById("wizard-origin");
  if (originEl) originEl.textContent = window.location.origin;

  // Step 1 — catalog
  const stepCatalog = document.getElementById("step-catalog");
  const stepLabel = document.getElementById("wizard-step-label");
  const simpleGrid = document.getElementById("catalog-simple");
  const advancedGrid = document.getElementById("catalog-advanced");
  const simpleEmpty = document.getElementById("catalog-simple-empty");
  const catalogStatus = document.getElementById("catalog-status");
  const searchInput = document.getElementById("catalog-search");
  const nextBtn = document.getElementById("wizard-next");
  const cancelBtn = document.getElementById("wizard-cancel");

  // Step 2 — credential
  const stepCredential = document.getElementById("step-credential");
  const credentialTitle = document.getElementById("credential-title");
  const credentialSubtitle = document.getElementById("credential-subtitle");
  const credentialLabelInput = document.getElementById("credential-label");
  const credentialValueInput = document.getElementById("credential-value");
  const credentialReveal = document.getElementById("credential-reveal");
  const credentialHint = document.getElementById("credential-hint");
  const credentialStatus = document.getElementById("credential-status");
  const credentialBack = document.getElementById("credential-back");
  const credentialSubmit = document.getElementById("credential-submit");

  // Step 3 — confirmation
  const stepConfirm = document.getElementById("step-confirm");
  const confirmSlug = document.getElementById("confirm-slug");
  const confirmLabel = document.getElementById("confirm-label");
  const confirmProxyUrl = document.getElementById("confirm-proxy-url");
  const confirmCurl = document.getElementById("confirm-curl");
  const copyProxyBtn = document.getElementById("copy-proxy-url");
  const copyCurlBtn = document.getElementById("copy-curl");
  const confirmStatus = document.getElementById("confirm-status");
  const doneBtn = document.getElementById("wizard-done");

  let catalog = [];       // raw catalog entries from backend
  let selection = null;   // catalog entry currently highlighted
  let createdKey = null;  // result of POST /keys
  let finished = false;   // once Done/Cancel clicked, don't fire again

  // ---- helpers ----

  async function proxyFetch(method, path, body) {
    const headers = { "x-wizard-csrf": CSRF };
    const opts = { method, headers, credentials: "omit" };
    if (body !== undefined) {
      headers["content-type"] = "application/json";
      opts.body = JSON.stringify(body);
    }
    return fetch(path, opts);
  }

  async function proxyJson(method, path, body) {
    const res = await proxyFetch(method, path, body);
    const text = await res.text().catch(() => "");
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch (_) { data = text; }
    if (!res.ok) {
      const err = new Error(`HTTP ${res.status} · ${typeof data === "string" ? data.slice(0, 300) : JSON.stringify(data).slice(0, 300)}`);
      err.status = res.status;
      err.body = data;
      throw err;
    }
    return data;
  }

  function setStatus(el, msg, cls) {
    if (!el) return;
    el.textContent = msg || "";
    el.className = "wizard-status" + (cls ? " " + cls : "");
  }

  function isSimpleBearer(entry) {
    // Per docs/CLI_WIZARD_V2.md §3.3 — "Simple setup" includes catalog
    // entries that the SimpleKey form can fully handle: bearer/header
    // api_key auth with no gateway-URL requirement, no OAuth/device-code,
    // and no token-exchange multi-field setup.
    const pt = entry.provider_type || null;
    const allowedPT = pt === null || pt === "api_key";
    const isHttp = (entry.service_type || "http") === "http";
    const needsGateway = !!entry.requires_gateway_url;
    const authMethod = (entry.auth_method || "bearer").toLowerCase();
    const okAuth = authMethod === "bearer" || authMethod === "header";
    const hasMultiField = Array.isArray(entry.token_exchange_credential_fields)
      && entry.token_exchange_credential_fields.length > 0;
    return allowedPT && isHttp && !needsGateway && okAuth && !hasMultiField
      && (entry.requires_credential !== false);
  }

  function cardEl(entry) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "wizard-card";
    btn.setAttribute("role", "listitem");
    btn.dataset.slug = entry.slug;

    const title = document.createElement("div");
    title.className = "wizard-card-title";
    title.textContent = entry.name || entry.slug;
    btn.appendChild(title);

    const sub = document.createElement("div");
    sub.className = "wizard-card-sub";
    sub.textContent = (entry.description || "").slice(0, 140);
    btn.appendChild(sub);

    const meta = document.createElement("div");
    meta.className = "wizard-card-meta";
    const hint = entry.auth_method === "header" ? "header auth" : "paste API key";
    meta.textContent = hint;
    btn.appendChild(meta);

    btn.addEventListener("click", () => selectCard(entry));
    return btn;
  }

  function renderCatalog(entries, filter) {
    const f = (filter || "").trim().toLowerCase();
    simpleGrid.innerHTML = "";
    let shown = 0;
    for (const entry of entries) {
      if (!isSimpleBearer(entry)) continue;
      if (f && !(entry.slug.toLowerCase().includes(f) || (entry.name || "").toLowerCase().includes(f))) continue;
      simpleGrid.appendChild(cardEl(entry));
      shown += 1;
    }
    simpleEmpty.hidden = shown > 0;
  }

  function selectCard(entry) {
    selection = entry;
    for (const el of document.querySelectorAll(".wizard-card")) {
      el.classList.toggle("is-selected", el.dataset.slug === entry.slug);
    }
    nextBtn.disabled = false;
  }

  // ---- step transitions ----

  function showPanel(name) {
    stepCatalog.hidden = name !== "catalog";
    stepCredential.hidden = name !== "credential";
    stepConfirm.hidden = name !== "confirm";
    if (stepLabel) {
      stepLabel.textContent = {
        catalog: "Step 1 · pick a service",
        credential: "Step 2 · enter credential",
        confirm: "Step 3 · done",
      }[name] || "";
    }
  }

  function defaultLabelFor(entry) {
    // Use the catalog slug as the default label — the backend will
    // auto-suffix if the user already has one with that slug.
    return entry.slug;
  }

  function enterCredentialStep() {
    if (!selection) return;
    if (selection.slug === "__custom__") {
      setStatus(catalogStatus,
        "Custom / self-hosted form lands in M5. For now, use: nyxid service add <slug> --custom --endpoint-url <URL> --credential-env <VAR>",
        "error");
      return;
    }
    credentialTitle.textContent = `Connect ${selection.name || selection.slug}`;
    credentialSubtitle.textContent = (selection.description || "").slice(0, 200);
    credentialLabelInput.value = defaultLabelFor(selection);
    credentialValueInput.value = "";
    credentialValueInput.type = "password";
    credentialReveal.textContent = "show";
    setStatus(credentialStatus, "");
    const docsUrl = selection.api_key_url || selection.documentation_url;
    const instr = selection.api_key_instructions;
    if (instr) {
      credentialHint.textContent = instr;
    } else if (docsUrl) {
      credentialHint.textContent = `Paste the API key from ${docsUrl}`;
    } else {
      credentialHint.textContent = "Paste the key from the provider's dashboard.";
    }
    showPanel("credential");
    credentialLabelInput.focus();
  }

  async function submitCredential() {
    if (!selection) return;
    if (postInFlight) return;   // guard against double-click / Enter-spam
    const label = credentialLabelInput.value.trim();
    const credential = credentialValueInput.value.trim();
    if (!label) {
      setStatus(credentialStatus, "Label is required.", "error");
      credentialLabelInput.focus();
      return;
    }
    if (!credential) {
      setStatus(credentialStatus, "API key is required.", "error");
      credentialValueInput.focus();
      return;
    }
    postInFlight = true;
    credentialSubmit.disabled = true;
    credentialBack.disabled = true;
    setStatus(credentialStatus, `Creating '${label}'…`);

    try {
      const body = {
        service_slug: selection.slug,
        label,
        credential,
      };
      const data = await proxyJson("POST", "/api/proxy/api/v1/keys", body);
      createdKey = data || {};
      // Clear the raw credential from memory ASAP. The DOM input still
      // technically holds it until the page unloads; we clear that too.
      credentialValueInput.value = "";
      renderConfirm(createdKey);
      showPanel("confirm");
    } catch (err) {
      setStatus(credentialStatus,
        `Couldn't create service: ${err.message}`,
        "error");
    } finally {
      postInFlight = false;
      credentialSubmit.disabled = false;
      credentialBack.disabled = false;
    }
  }

  function renderConfirm(key) {
    // KeyResponse from POST /api/v1/keys is flat: { slug, label, endpoint_url, ... }
    // It does NOT include proxy_url (Codex review finding). We synthesize
    // the proxy URL from the base_url injected into the HTML at render time.
    const slug = key.slug || selection.slug;
    const label = key.label || selection.name || slug;
    const proxyUrl = BASE_URL
      ? `${BASE_URL}/api/v1/proxy/s/${slug}/`
      : `/api/v1/proxy/s/${slug}/`;
    confirmSlug.textContent = slug;
    confirmLabel.textContent = label;
    confirmProxyUrl.textContent = proxyUrl;
    confirmCurl.textContent =
      `curl ${proxyUrl}<api-path> \\\n` +
      `  -H "Authorization: Bearer $NYX_KEY"\n` +
      `# e.g. <api-path> = v1/models for OpenAI-compatible providers`;
  }

  async function copyText(text, btn) {
    try {
      await navigator.clipboard.writeText(text);
      if (btn) {
        const prev = btn.textContent;
        btn.textContent = "Copied!";
        setTimeout(() => { btn.textContent = prev; }, 1200);
      }
    } catch (_) {
      // Clipboard requires a secure context in some browsers; 127.0.0.1
      // is treated as secure but fallback just in case.
    }
  }

  // ---- lifecycle ----

  async function onDone() {
    if (finished) return;
    finished = true;
    doneBtn.disabled = true;
    setStatus(confirmStatus, "Signalling CLI…");
    try {
      const slug = createdKey?.slug || null;
      const label = createdKey?.label || null;
      const res = await proxyFetch("POST", "/api/proxy/complete", {
        flow: FLOW,
        milestone: "M3",
        slug,
        label,
        // proxy_url is synthesized CLI-side in main.rs::print_wizard_summary
        // using the same base_url_root that rendered this page, so we send
        // null here rather than a half-built URL from the browser.
        proxy_url: null,
      });
      if (!res.ok) {
        setStatus(confirmStatus, `CLI rejected the completion signal (HTTP ${res.status}).`, "error");
        finished = false; doneBtn.disabled = false;
        return;
      }
      setStatus(confirmStatus, "Done. You can close this tab.", "success");
    } catch (err) {
      setStatus(confirmStatus, "Couldn't reach the CLI: " + err.message, "error");
      finished = false; doneBtn.disabled = false;
    }
  }

  async function onCancel() {
    if (finished) return;
    finished = true;
    cancelBtn.disabled = true;
    setStatus(catalogStatus, "Cancelling…");
    try {
      await proxyFetch("POST", "/api/proxy/cancel", {});
      setStatus(catalogStatus, "Cancelled. You can close this tab.", "success");
    } catch (err) {
      setStatus(catalogStatus, "Couldn't reach the CLI: " + err.message, "error");
    }
  }

  // ---- heartbeat ----

  const HEARTBEAT_INTERVAL_MS = 10_000;
  let heartbeatTimer = null;
  async function sendHeartbeat() {
    try { await proxyFetch("POST", "/api/proxy/heartbeat", {}); } catch (_) { /* tab teardown */ }
  }
  function startHeartbeats() {
    if (heartbeatTimer) return;
    sendHeartbeat();
    heartbeatTimer = setInterval(sendHeartbeat, HEARTBEAT_INTERVAL_MS);
  }
  function stopHeartbeats() {
    if (heartbeatTimer) { clearInterval(heartbeatTimer); heartbeatTimer = null; }
  }
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) stopHeartbeats(); else startHeartbeats();
  });
  window.addEventListener("beforeunload", () => {
    // Do NOT fire cancel-unload if a mutating POST is mid-flight. Tab close
    // after Connect-click but before response can race with upstream
    // /api/v1/keys, creating a real service while the CLI reports cancel.
    if (postInFlight) return;
    try {
      const payload = new Blob([JSON.stringify({ reason: "unload" })], { type: "application/json" });
      navigator.sendBeacon("/api/proxy/cancel-unload", payload);
    } catch (_) { /* ignore */ }
  });

  // ---- init ----

  async function loadCatalog() {
    setStatus(catalogStatus, "Loading catalog…");
    try {
      const data = await proxyJson("GET", "/api/proxy/api/v1/catalog?include_all=true");
      catalog = Array.isArray(data?.entries)
        ? data.entries
        : Array.isArray(data?.services)
          ? data.services
          : Array.isArray(data)
            ? data
            : [];
      renderCatalog(catalog, searchInput.value);
      const simpleCount = catalog.filter(isSimpleBearer).length;
      setStatus(catalogStatus,
        `${catalog.length} services in catalog · ${simpleCount} simple-bearer shown`);
    } catch (err) {
      setStatus(catalogStatus,
        "Couldn't load catalog: " + err.message
          + ". Check your session with `nyxid whoami`, or re-login with `nyxid login --base-url <URL>`.",
        "error");
    }
  }

  function wire() {
    nextBtn.addEventListener("click", enterCredentialStep);
    cancelBtn.addEventListener("click", onCancel);
    credentialBack.addEventListener("click", () => {
      credentialValueInput.value = "";
      setStatus(credentialStatus, "");
      showPanel("catalog");
    });
    credentialSubmit.addEventListener("click", (e) => { e.preventDefault(); submitCredential(); });
    credentialReveal.addEventListener("click", () => {
      if (credentialValueInput.type === "password") {
        credentialValueInput.type = "text";
        credentialReveal.textContent = "hide";
      } else {
        credentialValueInput.type = "password";
        credentialReveal.textContent = "show";
      }
    });
    // Enter on the form submits.
    document.getElementById("credential-form").addEventListener("submit", (e) => {
      e.preventDefault();
      submitCredential();
    });
    doneBtn.addEventListener("click", onDone);
    searchInput.addEventListener("input", () => renderCatalog(catalog, searchInput.value));
    advancedGrid.querySelector('[data-slug="__custom__"]').addEventListener("click", () => {
      selectCard({ slug: "__custom__", name: "Custom / self-hosted" });
    });
    copyProxyBtn.addEventListener("click", () => copyText(confirmProxyUrl.textContent, copyProxyBtn));
    copyCurlBtn.addEventListener("click", () => copyText(confirmCurl.textContent, copyCurlBtn));
  }

  wire();
  if (!document.hidden) startHeartbeats();
  loadCatalog();
})();
