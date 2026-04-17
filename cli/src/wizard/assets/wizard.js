(() => {
  "use strict";

  // ---- bootstrap ----

  const CSRF = document.querySelector('meta[name="wizard-csrf"]')?.getAttribute("content") || "";
  const FLOW = document.querySelector('meta[name="wizard-flow"]')?.getAttribute("content") || "ai-key";

  const originEl = document.getElementById("wizard-origin");
  if (originEl) originEl.textContent = window.location.origin;

  const stepCatalog = document.getElementById("step-catalog");
  const stepPlaceholder = document.getElementById("step-placeholder");
  const stepLabel = document.getElementById("wizard-step-label");
  const simpleGrid = document.getElementById("catalog-simple");
  const advancedGrid = document.getElementById("catalog-advanced");
  const simpleEmpty = document.getElementById("catalog-simple-empty");
  const catalogStatus = document.getElementById("catalog-status");
  const searchInput = document.getElementById("catalog-search");
  const nextBtn = document.getElementById("wizard-next");
  const cancelBtn = document.getElementById("wizard-cancel");
  const backBtn = document.getElementById("step2-back");
  const doneBtn = document.getElementById("wizard-done");
  const pickedSlugEl = document.getElementById("picked-slug");
  const placeholderStatus = document.getElementById("placeholder-status");

  let catalog = [];       // raw catalog entries from backend
  let selection = null;   // catalog entry (or { slug: "__custom__" }) currently highlighted
  let finished = false;   // once Done/Cancel clicked, don't fire again

  // ---- helpers ----

  async function proxyFetch(method, path, body) {
    const headers = { "x-wizard-csrf": CSRF };
    const opts = { method, headers, credentials: "omit" };
    if (body !== undefined) {
      headers["content-type"] = "application/json";
      opts.body = JSON.stringify(body);
    }
    const res = await fetch(path, opts);
    return res;
  }

  async function proxyJson(method, path, body) {
    const res = await proxyFetch(method, path, body);
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      throw new Error(`${method} ${path}: HTTP ${res.status} ${text.slice(0, 200)}`);
    }
    const ct = res.headers.get("content-type") || "";
    if (ct.includes("application/json")) return res.json();
    return res.text();
  }

  function setStatus(el, msg, cls) {
    if (!el) return;
    el.textContent = msg || "";
    el.className = "wizard-status" + (cls ? " " + cls : "");
  }

  function isSimpleBearer(entry) {
    // Per docs/CLI_WIZARD_V2.md §3.3 — "Simple setup" includes catalog
    // entries that the SimpleKey form can fully handle: bearer-style
    // api_key auth with no gateway-URL requirement, no OAuth/device-code,
    // and no token-exchange multi-field setup.
    const pt = entry.provider_type || null; // null == plain HTTP bearer per catalog schema
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
    // Keep the static Custom… card in advancedGrid; we don't rebuild it.
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

  function goToStep(n) {
    stepCatalog.hidden = n !== 1;
    stepPlaceholder.hidden = n !== 2;
    if (stepLabel) {
      stepLabel.textContent = n === 1 ? "Step 1 · pick a service" : "Step 2 · placeholder (M3 fills this in)";
    }
  }

  function onNext() {
    if (!selection) return;
    if (pickedSlugEl) {
      pickedSlugEl.textContent = selection.slug === "__custom__"
        ? "Custom / self-hosted"
        : (selection.name ? `${selection.name} (${selection.slug})` : selection.slug);
    }
    goToStep(2);
  }

  function onBack() {
    goToStep(1);
  }

  // ---- lifecycle ----

  async function onDone() {
    if (finished) return;
    finished = true;
    doneBtn.disabled = true;
    backBtn.disabled = true;
    setStatus(placeholderStatus, "Signalling CLI…");
    try {
      const res = await proxyFetch("POST", "/api/proxy/complete", {
        flow: FLOW,
        milestone: "M2",
        selected_slug: selection ? selection.slug : null,
      });
      if (!res.ok) {
        setStatus(placeholderStatus, "CLI rejected the completion signal (HTTP " + res.status + ").", "error");
        finished = false;
        doneBtn.disabled = false;
        backBtn.disabled = false;
        return;
      }
      setStatus(placeholderStatus, "Done. You can close this tab and return to your terminal.", "success");
    } catch (err) {
      setStatus(placeholderStatus, "Couldn't reach the CLI: " + err.message, "error");
      finished = false;
      doneBtn.disabled = false;
      backBtn.disabled = false;
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
    try {
      await proxyFetch("POST", "/api/proxy/heartbeat", {});
    } catch (_) { /* tab teardown, ignore */ }
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
      // Backend currently returns { entries: [...] }. Tolerate { services: [...] }
      // and bare arrays so catalog schema tweaks don't break the wizard.
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
      // If we reached this page, base_url + token resolution already
      // succeeded in the CLI. A failure here means the backend itself
      // returned an error (network blip, 401 token expired, 5xx).
      setStatus(catalogStatus,
        "Couldn't load catalog: " + err.message
          + ". If this keeps happening, check your session with `nyxid whoami` "
          + "or re-login with `nyxid login --base-url <URL>`.",
        "error");
    }
  }

  function wire() {
    nextBtn.addEventListener("click", onNext);
    cancelBtn.addEventListener("click", onCancel);
    backBtn.addEventListener("click", onBack);
    doneBtn.addEventListener("click", onDone);
    searchInput.addEventListener("input", () => renderCatalog(catalog, searchInput.value));
    // Wire the static Custom… card.
    advancedGrid.querySelector('[data-slug="__custom__"]').addEventListener("click", () => {
      selectCard({ slug: "__custom__", name: "Custom / self-hosted" });
    });
  }

  wire();
  if (!document.hidden) startHeartbeats();
  loadCatalog();
})();
