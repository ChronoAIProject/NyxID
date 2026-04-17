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

  // Match the frontend `AddKeyDialog` catalog grid: show ALL catalog
  // entries, badge by flow shape, and route to the right sub-flow at
  // the form step. No hidden section.
  function flowShapeOf(entry) {
    if ((entry.service_type || "http") === "ssh") return "ssh";
    const pt = entry.provider_type || null;
    if (pt === "oauth2") return "oauth";
    if (pt === "device_code") return "device-code";
    if (entry.requires_credential === false) return "no-auth";
    if (Array.isArray(entry.token_exchange_credential_fields)
        && entry.token_exchange_credential_fields.length > 0) {
      return "token-exchange";
    }
    if (entry.requires_gateway_url) return "gateway-url";
    return "paste-key";
  }

  function shapeLabel(shape, entry) {
    switch (shape) {
      case "no-auth": return "1-click connect";
      case "gateway-url": return "URL + API key";
      case "token-exchange":
        return `${(entry.token_exchange_credential_fields || []).length} fields`;
      case "oauth": return "OAuth sign-in";
      case "device-code": return "device code";
      case "ssh": return "SSH cert";
      default: return "paste API key";
    }
  }

  function isWizardSupported(shape) {
    // Shapes the wizard form can fully complete today. Others get a
    // "use the scripted CLI for now" fallback in Step 2 (with a
    // copyable command) — they still show in Step 1 so the user sees
    // what's available.
    return shape === "paste-key" || shape === "gateway-url"
        || shape === "no-auth" || shape === "token-exchange";
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
    const shape = flowShapeOf(entry);
    meta.textContent = shapeLabel(shape, entry);
    if (!isWizardSupported(shape)) {
      btn.classList.add("wizard-card-disabled");
      btn.title = "Wizard support coming in a later PR — clickable to see the command you can use today.";
    }
    btn.appendChild(meta);

    btn.addEventListener("click", () => selectCard(entry));
    return btn;
  }

  function renderCatalog(entries, filter) {
    const f = (filter || "").trim().toLowerCase();
    simpleGrid.innerHTML = "";
    let shown = 0;
    // Show every catalog entry; the card's meta badge tells the user
    // what flow shape the service uses. Matches the frontend's
    // AddKeyDialog CatalogGrid behaviour.
    for (const entry of entries) {
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
        "Custom / self-hosted form lands in M5. For now, use: nyxid service add --custom --endpoint-url <URL> --credential-env <VAR> --label <LABEL>",
        "error");
      return;
    }
    const shape = flowShapeOf(selection);
    credentialTitle.textContent = `Connect ${selection.name || selection.slug}`;
    credentialSubtitle.textContent = (selection.description || "").slice(0, 200);
    setStatus(credentialStatus, "");

    // Build the form body dynamically from the catalog entry's shape.
    renderCredentialFormFields(selection, shape);
    showPanel("credential");
    // focus first real input
    const first = credentialFieldsEl.querySelector("input,textarea");
    if (first) first.focus();
  }

  // Container for dynamically rendered credential fields (varies per
  // flow shape). Replaces the old hard-coded label+password row.
  const credentialFieldsEl = document.getElementById("credential-fields");
  const credentialSubmitWrap = document.getElementById("credential-submit-wrap");

  function renderCredentialFormFields(entry, shape) {
    credentialFieldsEl.innerHTML = "";

    // Label is required on every shape (backend enforces).
    credentialFieldsEl.appendChild(fieldEl({
      id: "f-label", label: "Label", type: "text",
      value: defaultLabelFor(entry),
      hint: "Shown everywhere in the CLI and web UI.",
    }));

    // Fallback UI for shapes the wizard can't drive end-to-end yet.
    if (!isWizardSupported(shape)) {
      credentialFieldsEl.appendChild(unsupportedNotice(entry, shape));
      credentialSubmitWrap.hidden = true;
      return;
    }
    credentialSubmitWrap.hidden = false;

    if (shape === "gateway-url") {
      credentialFieldsEl.appendChild(fieldEl({
        id: "f-endpoint-url", label: "Gateway URL", type: "text",
        required: true,
        hint: "The URL of your self-hosted instance (e.g. https://openclaw.mycompany.com).",
      }));
      credentialFieldsEl.appendChild(pasteKeyField(entry));
    } else if (shape === "token-exchange") {
      const fields = entry.token_exchange_credential_fields || [];
      for (let i = 0; i < fields.length; i++) {
        const f = fields[i];
        credentialFieldsEl.appendChild(fieldEl({
          id: `f-tx-${i}`,
          label: f.label || f.name,
          type: f.secret ? "password" : "text",
          placeholder: f.placeholder || "",
          required: true,
          hint: f.description || "",
          secret: !!f.secret,
          name: f.name,
        }));
      }
    } else if (shape === "no-auth") {
      credentialFieldsEl.appendChild(noCredentialNotice());
    } else {
      // "paste-key" — simple bearer/header/path/query/bot_bearer
      credentialFieldsEl.appendChild(pasteKeyField(entry));
    }
  }

  function fieldEl(spec) {
    const wrap = document.createElement("label");
    wrap.className = "wizard-field";
    const lbl = document.createElement("span");
    lbl.className = "wizard-field-label";
    lbl.textContent = spec.label + (spec.required === false ? " (optional)" : "");
    wrap.appendChild(lbl);

    if (spec.secret) {
      const row = document.createElement("div");
      row.className = "wizard-input-row";
      const input = document.createElement("input");
      input.id = spec.id;
      input.type = "password";
      input.autocomplete = "off";
      input.spellcheck = false;
      if (spec.placeholder) input.placeholder = spec.placeholder;
      if (spec.value) input.value = spec.value;
      if (spec.required !== false) input.required = true;
      if (spec.name) input.dataset.name = spec.name;
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.className = "wizard-input-toggle";
      toggle.textContent = "show";
      toggle.setAttribute("aria-label", "show/hide");
      toggle.addEventListener("click", () => {
        if (input.type === "password") { input.type = "text"; toggle.textContent = "hide"; }
        else { input.type = "password"; toggle.textContent = "show"; }
      });
      row.appendChild(input); row.appendChild(toggle);
      wrap.appendChild(row);
    } else {
      const input = document.createElement("input");
      input.id = spec.id;
      input.type = spec.type || "text";
      input.autocomplete = "off";
      input.spellcheck = false;
      if (spec.placeholder) input.placeholder = spec.placeholder;
      if (spec.value) input.value = spec.value;
      if (spec.required !== false) input.required = true;
      if (spec.name) input.dataset.name = spec.name;
      wrap.appendChild(input);
    }
    if (spec.hint) {
      const hint = document.createElement("span");
      hint.className = "wizard-field-hint";
      hint.textContent = spec.hint;
      wrap.appendChild(hint);
    }
    return wrap;
  }

  function pasteKeyField(entry) {
    const docsUrl = entry.api_key_url || entry.documentation_url;
    const instr = entry.api_key_instructions;
    const hint = instr || (docsUrl ? `Paste the API key from ${docsUrl}` : "Paste the key from the provider's dashboard.");
    return fieldEl({
      id: "f-credential",
      label: "API key",
      type: "password",
      secret: true,
      required: true,
      hint,
    });
  }

  function noCredentialNotice() {
    const panel = document.createElement("div");
    panel.className = "wizard-info-panel";
    panel.textContent = "This service doesn't require a credential. "
      + "Click Connect to wire up the routing and you're done.";
    return panel;
  }

  function unsupportedNotice(entry, shape) {
    const msg = {
      "oauth": `${entry.name} uses OAuth sign-in — wizard support lands in a later PR. For now, run:`,
      "device-code": `${entry.name} uses device code — wizard support lands in a later PR. For now, run:`,
      "ssh": `${entry.name} is an SSH service — use \`nyxid service add-ssh\` instead. For now, run:`,
    }[shape] || "Wizard support coming. For now, run:";
    const cmd = shape === "ssh"
      ? `nyxid service add-ssh --label <LABEL> --host <HOST> --via-node <NODE>`
      : shape === "oauth"
      ? `nyxid service add ${entry.slug} --oauth`
      : shape === "device-code"
      ? `nyxid service add ${entry.slug} --device-code`
      : `nyxid service add ${entry.slug} --credential-env VAR --label <LABEL>`;

    const wrap = document.createElement("div");
    wrap.className = "wizard-info-panel";
    const p = document.createElement("p");
    p.textContent = msg;
    p.style.margin = "0 0 0.5rem";
    wrap.appendChild(p);
    const pre = document.createElement("pre");
    pre.className = "wizard-code";
    pre.style.margin = "0";
    pre.textContent = cmd;
    wrap.appendChild(pre);
    const copy = document.createElement("button");
    copy.type = "button";
    copy.className = "wizard-btn-tiny";
    copy.textContent = "Copy command";
    copy.style.marginTop = "0.5rem";
    copy.addEventListener("click", () => copyText(cmd, copy));
    wrap.appendChild(copy);
    return wrap;
  }

  function readField(id) {
    const el = document.getElementById(id);
    return el ? el.value.trim() : "";
  }

  function buildCreateBody() {
    if (!selection) return null;
    const shape = flowShapeOf(selection);
    const label = readField("f-label");
    if (!label) return { error: "Label is required." };
    const body = { service_slug: selection.slug, label };

    if (shape === "no-auth") {
      return { body };
    }
    if (shape === "gateway-url") {
      const endpointUrl = readField("f-endpoint-url");
      const credential = readField("f-credential");
      if (!endpointUrl) return { error: "Gateway URL is required." };
      if (!credential) return { error: "API key is required." };
      return { body: { ...body, endpoint_url: endpointUrl, credential } };
    }
    if (shape === "token-exchange") {
      const fields = selection.token_exchange_credential_fields || [];
      const creds = {};
      for (let i = 0; i < fields.length; i++) {
        const val = readField(`f-tx-${i}`);
        if (!val) return { error: `${fields[i].label || fields[i].name} is required.` };
        creds[fields[i].name] = val;
      }
      // Backend's /keys accepts the multi-field token-exchange as a
      // JSON-encoded credential string. See service.rs for the same
      // pattern used by the existing CLI.
      return { body: { ...body, credential: JSON.stringify(creds) } };
    }
    // default "paste-key"
    const credential = readField("f-credential");
    if (!credential) return { error: "API key is required." };
    return { body: { ...body, credential } };
  }

  function wipeCredentialInputs() {
    // Defence in depth: clear all inputs after submit so the pasted key
    // isn't sitting in the DOM until page unload.
    credentialFieldsEl.querySelectorAll("input").forEach(el => { el.value = ""; });
  }

  async function submitCredential() {
    if (!selection) return;
    if (postInFlight) return;
    const built = buildCreateBody();
    if (!built) return;
    if (built.error) {
      setStatus(credentialStatus, built.error, "error");
      return;
    }
    postInFlight = true;
    credentialSubmit.disabled = true;
    credentialBack.disabled = true;
    setStatus(credentialStatus, `Creating '${built.body.label}'…`);
    try {
      const data = await proxyJson("POST", "/api/proxy/api/v1/keys", built.body);
      createdKey = data || {};
      wipeCredentialInputs();
      renderConfirm(createdKey);
      showPanel("confirm");
    } catch (err) {
      setStatus(credentialStatus, `Couldn't create service: ${err.message}`, "error");
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
      wipeCredentialInputs();
      setStatus(credentialStatus, "");
      showPanel("catalog");
    });
    credentialSubmit.addEventListener("click", (e) => { e.preventDefault(); submitCredential(); });
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
