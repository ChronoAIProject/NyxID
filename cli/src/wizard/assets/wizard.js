(() => {
  "use strict";

  // ---- bootstrap ----

  const CSRF = document.querySelector('meta[name="wizard-csrf"]')?.getAttribute("content") || "";
  const FLOW = document.querySelector('meta[name="wizard-flow"]')?.getAttribute("content") || "ai-key";
  const BASE_URL = (document.querySelector('meta[name="wizard-base-url"]')?.getAttribute("content") || "").replace(/\/+$/, "");

  // Prefill from URL query — CLI passed these via `nyxid service add <slug>
  // --label X --via-node Y` etc. Missing values are normal; the form falls
  // back to its own defaults.
  const PARAMS = new URLSearchParams(window.location.search);
  const PREFILL = {
    slug: PARAMS.get("slug") || null,
    label: PARAMS.get("label") || null,
    viaNode: PARAMS.get("via_node") || null,
    endpointUrl: PARAMS.get("endpoint_url") || null,
  };

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
  let selectionDetail = null; // full catalog-detail fetch (credential_mode, provider_config_id, docs)
  let createdKey = null;  // result of POST /keys
  let finished = false;   // once Done/Cancel clicked, don't fire again
  let oauthCredentialsSet = false; // flipped after PUT /providers/:id/credentials succeeds
  // Tracks an in-flight OAuth / device-code placeholder key so we can
  // DELETE it if the user cancels, hits Back, or closes the tab before
  // authorization completes. Cleared once the key transitions to active.
  let pendingPlaceholderKeyId = null;

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
    let res;
    try {
      res = await proxyFetch(method, path, body);
    } catch (networkErr) {
      // fetch() throws TypeError ("failed to fetch") only when the
      // request can't complete at all. On same-origin loopback the
      // realistic cause is the wizard server died (CLI Ctrl-C'd or
      // exited). Translate into something the user can act on.
      // eslint-disable-next-line no-console
      console.error(`[wizard] ${method} ${path} — network failure`, networkErr);
      const msg = "Can't reach the wizard server. Is the `nyxid` CLI still "
        + "running? Close this tab and re-run `nyxid service add`.";
      const err = new Error(msg);
      err.cause = networkErr;
      err.network = true;
      throw err;
    }
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

  function showErrorBanner(msg) {
    const el = document.getElementById("credential-error");
    if (!el) return;
    el.textContent = msg || "";
    el.hidden = !msg;
  }
  function hideErrorBanner() { showErrorBanner(""); }

  // Match the frontend `AddKeyDialog` catalog grid: show ALL catalog
  // entries, badge by flow shape, and route to the right sub-flow at
  // the form step. No hidden section.
  function flowShapeOf(entry) {
    // The "Custom / self-hosted" card is synthesised client-side, not a
    // real catalog entry — treat its slug as a dedicated shape so the
    // form renderer and submit builder can branch on it.
    if (entry.slug === "__custom__") return "custom";
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
    // All non-SSH flows are wizard-supported. SSH needs certificate
    // issuance (different command: `nyxid service add-ssh`) and isn't
    // part of this wizard scope.
    return shape !== "ssh";
  }

  function cardEl(entry) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "wizard-card";
    btn.setAttribute("role", "listitem");
    btn.dataset.slug = entry.slug;

    const shape = flowShapeOf(entry);

    // Type badge for flows the wizard can't drive end-to-end yet.
    // Cards remain fully interactive — the badge is a visual tag, not
    // a disabled marker. Matches the frontend's AddKeyDialog catalog.
    if (!isWizardSupported(shape)) {
      const badge = document.createElement("span");
      badge.className = "wizard-card-badge";
      badge.dataset.shape = shape;
      badge.textContent = {
        "oauth": "OAuth",
        "device-code": "Device code",
        "ssh": "SSH",
      }[shape] || shape;
      btn.appendChild(badge);
    }

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
    meta.textContent = shapeLabel(shape, entry);
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

  // Total step count. Keep consistent across panels so the progress
  // indicator doesn't wiggle when the form branches into a fallback.
  const STEP_TOTAL = 3;
  const STEP_LABELS = {
    catalog:    `Step 1 of ${STEP_TOTAL} · pick a service`,
    credential: `Step 2 of ${STEP_TOTAL} · enter credential`,
    confirm:    `Step 3 of ${STEP_TOTAL} · done`,
  };

  function showPanel(name) {
    stepCatalog.hidden = name !== "catalog";
    stepCredential.hidden = name !== "credential";
    stepConfirm.hidden = name !== "confirm";
    if (stepLabel) {
      stepLabel.textContent = STEP_LABELS[name] || "";
    }
  }

  function defaultLabelFor(entry) {
    // Use the catalog slug as the default label — the backend will
    // auto-suffix if the user already has one with that slug.
    return entry.slug;
  }

  async function enterCredentialStep() {
    if (!selection) return;
    const shape = flowShapeOf(selection);
    if (shape === "custom") {
      credentialTitle.textContent = "Custom service";
      credentialSubtitle.textContent =
        "Paste your endpoint URL and credential. This creates a service that "
        + "proxies to anything you can hit over HTTP.";
    } else {
      credentialTitle.textContent = `Connect ${selection.name || selection.slug}`;
      credentialSubtitle.textContent = (selection.description || "").slice(0, 200);
    }
    setStatus(credentialStatus, "");
    hideErrorBanner();
    oauthCredentialsSet = false;
    selectionDetail = null;

    // For OAuth we need the full catalog entry (credential_mode,
    // documentation_url, provider_config_id) to decide whether to show
    // the client_id/secret sub-step first.
    if (shape === "oauth") {
      showPanel("credential");
      setStatus(credentialStatus, "Loading provider details…");
      try {
        selectionDetail = await proxyJson("GET",
          `/api/proxy/api/v1/catalog/${encodeURIComponent(selection.slug)}`);
      } catch (err) {
        showErrorBanner(`Couldn't load provider details: ${err.message}`);
        return;
      }
      setStatus(credentialStatus, "");
    }

    // Build the form body dynamically from the catalog entry's shape.
    renderCredentialFormFields(selection, shape);
    showPanel("credential");
    const first = credentialFieldsEl.querySelector("input,textarea");
    if (first) first.focus();
  }

  // Container for dynamically rendered credential fields (varies per
  // flow shape). Replaces the old hard-coded label+password row.
  const credentialFieldsEl = document.getElementById("credential-fields");
  const credentialSubmitWrap = document.getElementById("credential-submit-wrap");

  function renderCredentialFormFields(entry, shape) {
    credentialFieldsEl.innerHTML = "";

    // The purple accent wraps only the inputs the user has to fill.
    // Info panels and instructional blocks (OAuth intro, device-code
    // panel, no-auth notice, unsupported fallback) are appended to
    // `credentialFieldsEl` directly so they sit outside the accent.
    const inputGroup = document.createElement("div");
    inputGroup.className = "wizard-input-group";

    // Label is required on every shape (backend enforces). If the CLI
    // passed a --label, prefer that over the slug-derived default. For
    // the Custom form the synthetic `__custom__` slug isn't a useful
    // default — leave the field blank so the user picks their own.
    inputGroup.appendChild(fieldEl({
      id: "f-label", label: "Label", type: "text",
      value: PREFILL.label || (shape === "custom" ? "" : defaultLabelFor(entry)),
      hint: "Shown everywhere in the CLI and web UI.",
    }));

    // Fallback UI for shapes the wizard can't drive end-to-end yet.
    if (!isWizardSupported(shape)) {
      credentialFieldsEl.appendChild(inputGroup);
      credentialFieldsEl.appendChild(unsupportedNotice(entry, shape));
      credentialSubmitWrap.hidden = true;
      return;
    }
    credentialSubmitWrap.hidden = false;

    if (shape === "oauth") {
      const mode = (selectionDetail?.credential_mode || "system").toLowerCase();
      if ((mode === "user" || mode === "both") && !oauthCredentialsSet) {
        // Intro panel sits outside the accent; the two credential
        // fields sit inside so the accent covers both.
        credentialFieldsEl.appendChild(oauthCredentialsIntro(entry, selectionDetail));
        appendOauthCredentialFields(inputGroup);
        credentialFieldsEl.appendChild(inputGroup);
      } else {
        credentialFieldsEl.appendChild(inputGroup);
        credentialFieldsEl.appendChild(oauthInstructions(entry, selectionDetail));
      }
      return;
    }
    if (shape === "device-code") {
      // Device-code panel is where the user actually acts (copy code,
      // click Open) so it sits INSIDE the accent group alongside the
      // label — the purple bar stretches to cover the whole authorize
      // step. `renderDeviceCodePanel` / `renderDeviceCodeExpired` still
      // locate it via getElementById regardless of parent.
      inputGroup.appendChild(deviceCodeInstructions(entry));
      credentialFieldsEl.appendChild(inputGroup);
      return;
    }
    if (shape === "gateway-url") {
      inputGroup.appendChild(fieldEl({
        id: "f-endpoint-url", label: "Gateway URL", type: "text",
        value: PREFILL.endpointUrl || "",
        required: true,
        hint: "The URL of your self-hosted instance (e.g. https://openclaw.mycompany.com).",
      }));
      inputGroup.appendChild(pasteKeyField(entry));
    } else if (shape === "token-exchange") {
      const fields = entry.token_exchange_credential_fields || [];
      for (let i = 0; i < fields.length; i++) {
        const f = fields[i];
        inputGroup.appendChild(fieldEl({
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
      credentialFieldsEl.appendChild(inputGroup);
      credentialFieldsEl.appendChild(noCredentialNotice());
      return;
    } else if (shape === "custom") {
      appendCustomFormFields(inputGroup);
    } else {
      // "paste-key" — simple bearer/header/path/query/bot_bearer
      inputGroup.appendChild(pasteKeyField(entry));
    }
    credentialFieldsEl.appendChild(inputGroup);
  }

  // Custom / self-hosted form. Exposes the flag surface of
  // `nyxid service add --custom` as inputs. Intentionally rough per
  // CLI_WIZARD_V2.md §3.4b — polish is follow-up work.
  function appendCustomFormFields(root) {
    root.appendChild(fieldEl({
      id: "f-endpoint-url", label: "Endpoint URL", type: "text",
      value: PREFILL.endpointUrl || "",
      required: true,
      placeholder: "https://api.example.com",
      hint: "The base URL NyxID will proxy to.",
    }));
    root.appendChild(fieldEl({
      id: "f-credential", label: "API key / credential",
      type: "password", secret: true, required: true,
      hint: "Pasted once, encrypted at rest. Use 'user:pass' for basic auth.",
    }));
    root.appendChild(selectEl({
      id: "f-auth-method", label: "Auth method",
      value: "bearer",
      options: [
        { value: "bearer", label: "bearer (Authorization: Bearer …)" },
        { value: "header", label: "header (custom header)" },
        { value: "query",  label: "query (?key=…)" },
        { value: "basic",  label: "basic (Authorization: Basic …)" },
        { value: "none",   label: "none (no auth injection)" },
      ],
      hint: "How NyxID attaches the credential to outgoing requests.",
    }));
    root.appendChild(fieldEl({
      id: "f-auth-key-name", label: "Auth key name", type: "text",
      value: "Authorization", required: false,
      hint: "Header name for 'header', query parameter for 'query'. "
        + "Ignored for 'none'.",
    }));
    root.appendChild(fieldEl({
      id: "f-slug", label: "Custom slug", type: "text",
      required: false,
      placeholder: "auto-generated from label",
      hint: "URL segment at /proxy/s/<slug>/…. Leave blank to let "
        + "NyxID derive it from the label.",
    }));
    root.appendChild(fieldEl({
      id: "f-openapi-spec-url", label: "OpenAPI spec URL", type: "text",
      required: false,
      placeholder: "https://api.example.com/openapi.json",
      hint: "Optional. If provided, agents can discover individual "
        + "endpoints instead of only the generic proxy tool.",
    }));
  }

  function selectEl(spec) {
    const wrap = document.createElement("label");
    wrap.className = "wizard-field";
    const lbl = document.createElement("span");
    lbl.className = "wizard-field-label";
    lbl.textContent = spec.label;
    wrap.appendChild(lbl);
    const select = document.createElement("select");
    select.id = spec.id;
    select.className = "wizard-select";
    for (const opt of spec.options) {
      const o = document.createElement("option");
      o.value = opt.value;
      o.textContent = opt.label;
      if (opt.value === spec.value) o.selected = true;
      select.appendChild(o);
    }
    wrap.appendChild(select);
    if (spec.hint) {
      const hint = document.createElement("span");
      hint.className = "wizard-field-hint";
      hint.textContent = spec.hint;
      wrap.appendChild(hint);
    }
    return wrap;
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

  function oauthInstructions(entry, detail) {
    const panel = document.createElement("div");
    panel.className = "wizard-info-panel";
    const docsUrl = detail?.documentation_url || entry.documentation_url;
    panel.innerHTML = `
      <p style="margin:0 0 0.5rem"><strong>Sign in with ${escapeHTML(entry.name || entry.slug)}</strong></p>
      <p style="margin:0 0 0.5rem">
        Click <strong>Connect</strong>. A new browser tab will open at the
        provider's sign-in page. After you authorize, this wizard will
        automatically complete in the background.
      </p>
      <p style="margin:0" class="wizard-muted">Keep this tab open while the authorization runs.</p>
      ${docsUrl ? `<p style="margin:0.75rem 0 0"><a href="${escapeAttr(docsUrl)}" target="_blank" rel="noopener noreferrer" class="wizard-link">📖 How to set up ${escapeHTML(entry.name || entry.slug)} OAuth ↗</a></p>` : ""}
    `;
    return panel;
  }

  // OAuth credentials sub-step (for providers whose `credential_mode`
  // is "user" or "both"). Split into an info panel (rendered outside
  // the input-group accent) and the two credential fields (rendered
  // inside the accent). Mirrors frontend `OAuthCredentialsStep` in
  // add-key-dialog.tsx:1561-1667.
  function oauthCredentialsIntro(entry, detail) {
    const intro = document.createElement("div");
    intro.className = "wizard-info-panel";
    const docsUrl = detail?.documentation_url || entry.documentation_url;
    intro.innerHTML = `
      <p style="margin:0 0 0.5rem">
        <strong>${escapeHTML(entry.name || entry.slug)} needs your OAuth app credentials first.</strong>
      </p>
      <p style="margin:0 0 0.5rem" class="wizard-muted">
        Register an OAuth app on ${escapeHTML(entry.name || entry.slug)} (Developer
        Settings → OAuth Apps), then paste its Client ID and Client Secret
        below. The credentials are encrypted at rest on NyxID.
      </p>
      ${docsUrl ? `<p style="margin:0"><a href="${escapeAttr(docsUrl)}" target="_blank" rel="noopener noreferrer" class="wizard-link">📖 How to create an OAuth app ↗</a></p>` : ""}
    `;
    return intro;
  }

  function appendOauthCredentialFields(root) {
    root.appendChild(fieldEl({
      id: "f-client-id",
      label: "Client ID",
      type: "text",
      required: true,
      hint: "From the OAuth app you just created on the provider.",
    }));
    root.appendChild(fieldEl({
      id: "f-client-secret",
      label: "Client Secret",
      type: "password",
      secret: true,
      required: true,
      hint: "Never leaves your machine until Connect is clicked. Encrypted at rest on NyxID.",
    }));
  }

  function escapeAttr(s) {
    return (s || "").replace(/[&<>"']/g, c => ({
      "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
    }[c]));
  }

  function deviceCodeInstructions(entry) {
    const panel = document.createElement("div");
    panel.className = "wizard-info-panel";
    panel.id = "device-code-panel";
    panel.innerHTML = `
      <p style="margin:0 0 0.5rem"><strong>Device code authorization</strong></p>
      <p style="margin:0" class="wizard-muted">
        Click <strong>Connect</strong>. You'll get a short code to enter on
        ${escapeHTML(entry.name || entry.slug)}'s device-authorization page.
        The wizard polls automatically.
      </p>
    `;
    return panel;
  }

  function escapeHTML(s) {
    return (s || "").replace(/[&<>"']/g, c => ({
      "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
    }[c]));
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

    if (shape === "custom") {
      const endpointUrl = readField("f-endpoint-url");
      const credential = readField("f-credential");
      const authMethod = readField("f-auth-method") || "bearer";
      const authKeyName = readField("f-auth-key-name");
      const customSlug = readField("f-slug");
      const openapi = readField("f-openapi-spec-url");
      if (!endpointUrl) return { error: "Endpoint URL is required." };
      if (authMethod !== "none" && !credential) {
        return { error: "Credential is required for this auth method." };
      }
      const customBody = {
        label,
        endpoint_url: endpointUrl,
        auth_method: authMethod,
      };
      if (credential) customBody.credential = credential;
      if (authKeyName) customBody.auth_key_name = authKeyName;
      if (customSlug) customBody.slug = customSlug;
      if (openapi) customBody.openapi_spec_url = openapi;
      return { body: customBody };
    }

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
    const shape = flowShapeOf(selection);
    hideErrorBanner();
    if (shape === "oauth") {
      const mode = (selectionDetail?.credential_mode || "system").toLowerCase();
      // Sub-step A: persist the user's OAuth app credentials, then
      // re-render Step 2 with the sign-in panel.
      if ((mode === "user" || mode === "both") && !oauthCredentialsSet) {
        await submitOauthCredentials();
        return;
      }
      // Sub-step B: kick off the real OAuth sign-in + poll.
      await submitAuthFlow("oauth");
      return;
    }
    if (shape === "device-code") {
      await submitAuthFlow(shape);
      return;
    }
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

  // Sub-step A for OAuth with user-configured OAuth apps: PUT the
  // client_id + client_secret onto the provider entry, then re-render
  // the form as the sign-in panel. Mirrors the frontend's
  // OAuthCredentialsStep → OAuthStep transition.
  async function submitOauthCredentials() {
    const label = readField("f-label");
    const clientId = readField("f-client-id");
    const clientSecret = readField("f-client-secret");
    if (!label) { showErrorBanner("Label is required."); return; }
    if (!clientId) { showErrorBanner("Client ID is required."); return; }
    if (!clientSecret) { showErrorBanner("Client Secret is required."); return; }
    const providerId = selectionDetail?.provider_config_id;
    if (!providerId) { showErrorBanner("Catalog is missing provider_config_id for this service."); return; }

    postInFlight = true;
    credentialSubmit.disabled = true;
    credentialBack.disabled = true;
    setStatus(credentialStatus, "Saving OAuth app credentials…");

    try {
      await proxyJson("PUT",
        `/api/proxy/api/v1/providers/${encodeURIComponent(providerId)}/credentials`,
        { client_id: clientId, client_secret: clientSecret, label });
      oauthCredentialsSet = true;
      setStatus(credentialStatus, "");
      // Re-render Step 2 — now shows the sign-in panel with docs link.
      renderCredentialFormFields(selection, "oauth");
    } catch (err) {
      showErrorBanner(`Couldn't save OAuth app credentials: ${err.message}`);
      setStatus(credentialStatus, "");
    } finally {
      postInFlight = false;
      credentialSubmit.disabled = false;
      credentialBack.disabled = false;
    }
  }

  // ---- OAuth + device-code flows ----
  //
  // Ported from cli/src/commands/service.rs::run_oauth_add and
  // run_device_code_add. Three stages:
  //   1. Create placeholder /keys (status=pending_auth)
  //   2. Initiate auth (OAuth: GET /providers/:id/connect/oauth;
  //      device-code: POST /providers/:id/connect/device-code/initiate)
  //   3. Poll until the backend reports the credential is active

  async function submitAuthFlow(shape) {
    const label = readField("f-label");
    if (!label) {
      setStatus(credentialStatus, "Label is required.", "error");
      return;
    }
    postInFlight = true;
    credentialSubmit.disabled = true;
    credentialBack.disabled = true;
    setStatus(credentialStatus, "Fetching provider details…");

    try {
      // Stage 1 — catalog for provider_config_id.
      const catalog = await proxyJson("GET",
        `/api/proxy/api/v1/catalog/${encodeURIComponent(selection.slug)}`);
      const providerId = catalog?.provider_config_id;
      if (!providerId) {
        throw new Error("catalog entry has no provider_config_id");
      }

      // Stage 2 — placeholder key.
      setStatus(credentialStatus, `Creating placeholder '${label}'…`);
      const placeholder = await proxyJson("POST", "/api/proxy/api/v1/keys", {
        service_slug: selection.slug,
        label,
      });
      const keyId = placeholder?.id;
      if (!keyId) throw new Error("placeholder key has no id");

      // Track for cleanup on cancel/back/unload. Cleared on success below.
      pendingPlaceholderKeyId = keyId;

      // Short-circuit: when credential_mode=admin, the backend inherits
      // the admin's already-authorized credentials and returns the key
      // as status=active immediately — no OAuth round-trip needed.
      // Only skip for OAuth here. For device-code we always run the
      // initiate+poll dance even when a prior token exists, so the user
      // sees the code + verification URL and can re-authorize on the
      // provider (matches what they clicked "Connect" expecting).
      if (shape === "oauth" && placeholder?.status === "active") {
        pendingPlaceholderKeyId = null; // key is already valid; don't delete.
        createdKey = placeholder;
        renderConfirm(createdKey);
        showPanel("confirm");
        return;
      }

      // Stage 3 — initiate + poll.
      if (shape === "oauth") {
        await runOauthFlow(providerId, keyId);
      } else {
        await runDeviceCodeFlow(providerId, keyId);
      }

      // Stage 4 — fetch final key, confirm.
      const finalKey = await proxyJson("GET", `/api/proxy/api/v1/keys/${encodeURIComponent(keyId)}`);
      pendingPlaceholderKeyId = null; // flow completed; key is active.
      createdKey = finalKey || {};
      renderConfirm(createdKey);
      showPanel("confirm");
    } catch (err) {
      showErrorBanner(err.message || String(err));
      setStatus(credentialStatus, "");
    } finally {
      postInFlight = false;
      credentialSubmit.disabled = false;
      credentialBack.disabled = false;
    }
  }

  async function runOauthFlow(providerId, keyId) {
    setStatus(credentialStatus, "Opening provider sign-in in a new tab…");
    // Ask the backend for the authorization URL. `redirect_path` is
    // where NyxID's frontend sends the user *after* the OAuth callback
    // completes — nothing to do with the wizard tab, which polls
    // independently.
    const redirectPath = encodeURIComponent(`/keys/${keyId}`);
    const initiate = await proxyJson("GET",
      `/api/proxy/api/v1/providers/${encodeURIComponent(providerId)}/connect/oauth?redirect_path=${redirectPath}`);
    const authUrl = initiate?.authorization_url;
    if (!authUrl) throw new Error("provider did not return an authorization_url");

    // New tab so the wizard stays alive to poll.
    const w = window.open(authUrl, "_blank", "noopener,noreferrer");
    if (!w) {
      throw new Error("Browser blocked the popup. Copy this URL manually:\n" + authUrl);
    }
    await pollKeyUntilActive(keyId);
  }

  // Device-code flow with refresh support. Each time the user clicks
  // "Refresh code" (or the current code expires and they click the big
  // refresh button), we bump `deviceCodeGen`, kill the in-flight poll,
  // re-initiate, and restart polling. Only the latest generation's
  // success/failure resolves the outer promise.
  let deviceCodeGen = 0;
  let deviceCodeOuterResolve = null;
  let deviceCodeOuterReject = null;

  async function runDeviceCodeFlow(providerId, keyId) {
    return new Promise((resolve, reject) => {
      deviceCodeOuterResolve = resolve;
      deviceCodeOuterReject = reject;
      startDeviceCodeSession(providerId, keyId);
    });
  }

  async function startDeviceCodeSession(providerId, keyId) {
    const myGen = ++deviceCodeGen;
    const pollPath = `/api/proxy/api/v1/providers/${encodeURIComponent(providerId)}/connect/device-code/poll`;
    const initiatePath = `/api/proxy/api/v1/providers/${encodeURIComponent(providerId)}/connect/device-code/initiate`;

    setStatus(credentialStatus, "Requesting device code…");
    let init;
    try {
      init = await proxyJson("POST", initiatePath, {});
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("[wizard] device-code initiate failed", err);
      if (myGen === deviceCodeGen) deviceCodeOuterReject(err);
      return;
    }
    if (myGen !== deviceCodeGen) return; // superseded by refresh

    const userCode = init?.user_code || "-";
    const verificationUri = init?.verification_uri || init?.verification_url || "";
    const state = init?.state;
    let interval = Number(init?.interval) || 5;
    if (!state) {
      deviceCodeOuterReject(new Error("device-code initiate did not return state"));
      return;
    }

    renderDeviceCodePanel({
      userCode, verificationUri,
      onRefresh: () => startDeviceCodeSession(providerId, keyId),
    });
    // We're entering a long-polling state — release the Back button so
    // the user can bail if they change their mind. Submit stays disabled
    // (nothing to submit; Refresh handles re-entry).
    credentialBack.disabled = false;
    setStatus(credentialStatus, `Waiting for authorization (polling every ${interval}s)…`);

    const deadline = Date.now() + 10 * 60 * 1000; // 10 min per code
    while (Date.now() < deadline) {
      await sleep(interval * 1000);
      if (myGen !== deviceCodeGen) return; // refreshed — stop this loop

      let result;
      try { result = await proxyJson("POST", pollPath, { state }); }
      catch (_) { continue; /* transient; keep polling */ }
      if (myGen !== deviceCodeGen) return;

      const status = result?.status || "";
      if (status === "complete" || status === "authorized" || result?.access_token) {
        deviceCodeOuterResolve();
        return;
      }
      if (status === "expired") {
        renderDeviceCodeExpired({
          onRefresh: () => startDeviceCodeSession(providerId, keyId),
        });
        // Leave the outer promise unresolved — user can refresh for a
        // new code without leaving Step 2. They can also Cancel to bail.
        return;
      }
      if (status === "denied") {
        deviceCodeOuterReject(new Error("Authorization denied on the provider side."));
        return;
      }
      if (status === "slow_down") {
        interval = Number(result?.interval) || (interval + 5);
        setStatus(credentialStatus,
          `Provider asked us to slow down; polling every ${interval}s.`);
      }
    }
    if (myGen === deviceCodeGen) {
      deviceCodeOuterReject(new Error("Device code timed out after 10 minutes. Click Refresh to get a new code."));
    }
  }

  function renderDeviceCodePanel({ userCode, verificationUri, onRefresh }) {
    const panel = document.getElementById("device-code-panel");
    if (!panel) return;
    panel.innerHTML = `
      <p style="margin:0 0 0.75rem"><strong>Device code authorization</strong></p>
      <div style="display:grid;grid-template-columns:max-content 1fr;gap:0.5rem 1rem;margin:0 0 0.75rem">
        <span class="wizard-muted">Code</span>
        <div style="display:flex;gap:0.5rem;align-items:center;flex-wrap:wrap">
          <code style="font-size:1.125rem;padding:0.25rem 0.625rem;background:var(--ghost-hover);border-radius:6px">${escapeHTML(userCode)}</code>
          <button type="button" class="wizard-btn-tiny" id="dc-copy-code">Copy</button>
          <button type="button" class="wizard-btn-tiny" id="dc-refresh">↻ Refresh code</button>
        </div>
        <span class="wizard-muted">Visit</span>
        <div style="display:flex;gap:0.5rem;align-items:center">
          <code style="word-break:break-all">${escapeHTML(verificationUri)}</code>
          <button type="button" class="wizard-btn-tiny" id="dc-open-url">Open</button>
        </div>
      </div>
      <p style="margin:0" class="wizard-muted">
        Click <strong>Open</strong>, enter the code, and the wizard will
        complete automatically. Use <strong>Refresh code</strong> if the
        code expires or you want a fresh one.
      </p>
    `;
    document.getElementById("dc-copy-code")?.addEventListener("click", (e) =>
      copyText(userCode, e.currentTarget));
    document.getElementById("dc-open-url")?.addEventListener("click", () => {
      const w = window.open(verificationUri, "_blank", "noopener,noreferrer");
      if (!w) copyText(verificationUri, document.getElementById("dc-open-url"));
    });
    document.getElementById("dc-refresh")?.addEventListener("click", () => {
      setStatus(credentialStatus, "Refreshing device code…");
      onRefresh();
    });
  }

  function renderDeviceCodeExpired({ onRefresh }) {
    const panel = document.getElementById("device-code-panel");
    if (!panel) return;
    panel.innerHTML = `
      <p style="margin:0 0 0.5rem"><strong>Code expired</strong></p>
      <p style="margin:0 0 0.75rem" class="wizard-muted">
        The device code timed out before authorization completed. Click
        below to request a fresh code, or ← Back to pick a different
        service.
      </p>
      <button type="button" class="wizard-btn wizard-btn-primary" id="dc-refresh-big">
        ↻ Get a new code
      </button>
    `;
    setStatus(credentialStatus, "");
    credentialBack.disabled = false;  // user can bail from the expired state
    document.getElementById("dc-refresh-big")?.addEventListener("click", onRefresh);
  }

  async function pollKeyUntilActive(keyId) {
    const deadline = Date.now() + 5 * 60 * 1000; // 5 min, mirrors CLI's 150*2s
    while (Date.now() < deadline) {
      await sleep(2000);
      let key;
      try {
        key = await proxyJson("GET", `/api/proxy/api/v1/keys/${encodeURIComponent(keyId)}`);
      } catch (_) {
        continue; // transient
      }
      const status = key?.status || "";
      if (status === "active") return;
      if (status === "pending_auth") {
        setStatus(credentialStatus, "Waiting for you to complete authorization…");
        continue;
      }
      throw new Error(`Unexpected key status: ${status}`);
    }
    throw new Error("Timed out waiting for authorization. You can run `nyxid service list` later to check if it completed.");
  }

  function sleep(ms) {
    return new Promise(r => setTimeout(r, ms));
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

  function showOverlay(opts) {
    const overlay = document.getElementById("wizard-overlay");
    const card = overlay.querySelector(".wizard-overlay-card");
    const title = document.getElementById("overlay-title");
    const body = document.getElementById("overlay-body");
    const sub = document.getElementById("overlay-sub");
    const icon = document.getElementById("overlay-icon");
    title.textContent = opts.title;
    body.textContent = opts.body;
    sub.textContent = opts.sub || "";
    icon.textContent = opts.icon || "✓";
    card.classList.toggle("wizard-overlay-cancel", !!opts.cancel);
    card.classList.toggle("wizard-overlay-disconnect", !!opts.disconnect);
    overlay.hidden = false;
    overlay.setAttribute("aria-hidden", "false");
  }

  function hideOverlay() {
    const overlay = document.getElementById("wizard-overlay");
    if (!overlay) return;
    overlay.hidden = true;
    overlay.setAttribute("aria-hidden", "true");
  }

  function showDisconnectedOverlay() {
    showOverlay({
      disconnect: true,
      icon: "⚠",
      title: "Wizard disconnected",
      body: "The nyxid CLI is no longer running. Close this tab and re-run nyxid service add.",
      sub: "Your terminal has any partial state that was saved.",
    });
    // Kill any in-flight device-code poll or other long-running work so
    // their next tick exits silently instead of throwing into the UI.
    deviceCodeGen++;
    postInFlight = false;
    finished = true;
  }

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
      const displayLabel = label || slug || "Service";
      showOverlay({
        icon: "✓",
        title: `${displayLabel} complete`,
        body: "It is safe to close the browser now.",
        sub: "Your terminal has the details — switch back to it.",
      });
    } catch (err) {
      setStatus(confirmStatus, "Couldn't reach the CLI: " + err.message, "error");
      finished = false; doneBtn.disabled = false;
    }
  }

  // Best-effort conditional cleanup of an in-flight OAuth / device-code
  // placeholder key. Routed through the wizard-server-local abandon
  // endpoint, which does a server-side GET-then-DELETE so a key that
  // just transitioned to `active` while the user was bailing out can't
  // be revoked by mistake. Fire-and-forget — a failure here shouldn't
  // block cancel. Leaves `pendingPlaceholderKeyId = null` on return so
  // repeated calls are cheap no-ops. The server ALSO tracks observed
  // pending keys and drains them on shutdown, so this path is a nice-
  // to-have rather than the only line of defense.
  async function cleanupPendingPlaceholder() {
    const keyId = pendingPlaceholderKeyId;
    if (!keyId) return;
    pendingPlaceholderKeyId = null;
    try {
      await proxyFetch("POST", "/api/proxy/abandon-placeholder",
        { key_id: keyId });
    } catch (_) { /* best-effort; shutdown drain is the backstop */ }
  }

  async function onCancel() {
    if (finished) return;
    finished = true;
    cancelBtn.disabled = true;
    setStatus(catalogStatus, "Cancelling…");
    try {
      // Delete the placeholder BEFORE telling the CLI to shut down —
      // once shutdown runs, the local proxy stops accepting requests.
      await cleanupPendingPlaceholder();
      await proxyFetch("POST", "/api/proxy/cancel", {});
      showOverlay({
        cancel: true,
        icon: "✗",
        title: "Wizard cancelled",
        body: "No service was created. It is safe to close the browser now.",
      });
    } catch (err) {
      setStatus(catalogStatus, "Couldn't reach the CLI: " + err.message, "error");
    }
  }

  // ---- heartbeat + connection health ----
  //
  // Browser pings the CLI every 3 s. Two consecutive failures (~6 s) mean
  // the wizard server is gone — CLI exited, Ctrl-C'd, or crashed. We
  // show a full-screen disconnected overlay and stop polling; an
  // ephemeral port doesn't come back, so the only useful action is
  // "close this tab and re-run `nyxid service add`".

  const HEARTBEAT_INTERVAL_MS = 3_000;
  const DISCONNECT_AFTER_FAILURES = 2;
  let heartbeatTimer = null;
  let consecutiveHeartbeatFailures = 0;
  let disconnectedShown = false;

  async function sendHeartbeat() {
    if (disconnectedShown) return; // already gave up
    try {
      const res = await proxyFetch("POST", "/api/proxy/heartbeat", {});
      if (!res.ok) throw new Error(`heartbeat ${res.status}`);
      consecutiveHeartbeatFailures = 0;
    } catch (_) {
      consecutiveHeartbeatFailures += 1;
      if (consecutiveHeartbeatFailures >= DISCONNECT_AFTER_FAILURES) {
        disconnectedShown = true;
        stopHeartbeats();
        showDisconnectedOverlay();
      }
    }
  }
  function startHeartbeats() {
    if (heartbeatTimer) return;
    if (disconnectedShown) return;
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
    // Placeholder cleanup fires independently of the cancel-unload path
    // below — we want the stale `pending_auth` row gone even when a POST
    // is in flight. Route through the abandon-placeholder endpoint so
    // the server does the GET-then-conditional-DELETE (can't accidentally
    // revoke a key that just became active). `keepalive: true` lets the
    // POST complete after the page unloads. If the client-known keyId
    // is null because tab-close raced with the POST response, the
    // server's own pending-keys drain on shutdown is the backstop.
    if (pendingPlaceholderKeyId) {
      const keyId = pendingPlaceholderKeyId;
      pendingPlaceholderKeyId = null;
      try {
        fetch("/api/proxy/abandon-placeholder", {
          method: "POST",
          headers: {
            "x-wizard-csrf": CSRF,
            "content-type": "application/json",
          },
          body: JSON.stringify({ key_id: keyId }),
          keepalive: true,
          credentials: "omit",
        }).catch(() => {});
      } catch (_) { /* ignore */ }
    }
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
      const supportedCount = catalog.filter(e => isWizardSupported(flowShapeOf(e))).length;
      setStatus(catalogStatus,
        `${catalog.length} services · ${supportedCount} wizard-driven, ${catalog.length - supportedCount} copy-command fallback`);

      // Apply prefill from CLI flags: auto-select slug, auto-advance to Step 2.
      if (PREFILL.slug) {
        const match = catalog.find(e => e.slug === PREFILL.slug);
        if (match) {
          selectCard(match);
          enterCredentialStep();
        } else {
          setStatus(catalogStatus,
            `No catalog entry matches slug "${PREFILL.slug}". Pick one manually.`,
            "error");
        }
      }
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
      // Stop any in-flight device-code poll + invalidate its generation
      // so late-arriving poll responses can't fire outer resolve/reject.
      deviceCodeGen++;

      // For OAuth user/both mode: after credentials are saved we're on
      // sub-step B (sign-in). Back should return to sub-step A (edit
      // credentials), not bail to the catalog.
      if (selection
          && flowShapeOf(selection) === "oauth"
          && oauthCredentialsSet) {
        oauthCredentialsSet = false;
        hideErrorBanner();
        setStatus(credentialStatus, "");
        renderCredentialFormFields(selection, "oauth");
        const first = credentialFieldsEl.querySelector("input,textarea");
        if (first) first.focus();
        return;
      }
      // Otherwise: back to Step 1. If we created a placeholder key for
      // an abandoned OAuth / device-code flow, delete it so the user
      // doesn't accumulate half-authorized rows in `nyxid service list`.
      void cleanupPendingPlaceholder();
      wipeCredentialInputs();
      hideErrorBanner();
      setStatus(credentialStatus, "");
      selectionDetail = null;
      oauthCredentialsSet = false;
      postInFlight = false;
      credentialSubmit.disabled = false;
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
