importScripts("policy.js", "browser-background.js");

(() => {
  let port;
  let active = false;
  let retryMs = 500;
  const seen = new Map();

  async function fill(request) {
    const invalid = NyxIdFillerPolicy.validate(request);
    if (invalid) return { status: "refused", reason: invalid };
    if (!NyxIdFillerPolicy.valueAllowed(request.value)) return { status: "refused", reason: "invalid_value" };
    const now = Date.now();
    for (const [nonce, expiry] of seen) if (expiry <= now) seen.delete(nonce);
    if (seen.has(request.nonce)) return { status: "refused", reason: "replayed" };
    if (seen.size >= 512) return { status: "refused", reason: "busy" };
    seen.set(request.nonce, request.expires_at_ms);
    const tabs = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
    if (tabs.length !== 1 || !tabs[0].id) return { status: "refused", reason: "not_focused" };
    const tabId = tabs[0].id;
    const frames = await chrome.webNavigation.getAllFrames({ tabId });
    const candidates = (frames || []).filter(frame => {
      try { return request.allowed_origins.includes(new URL(frame.url).origin); }
      catch { return false; }
    });
    if (!candidates.length) return { status: "refused", reason: "origin_mismatch" };
    if (candidates.length > 64) return { status: "refused", reason: "too_many_frames" };
    // Probe without the value. Only the one focused frame can receive a fill.
    const probe = {
      operation: "probe", nonce: request.nonce, expires_at_ms: request.expires_at_ms,
      field: request.field, allowed_origins: request.allowed_origins,
    };
    const replies = await Promise.all(candidates.map(async frame => {
      try {
        const response = await chrome.tabs.sendMessage(tabId, probe, { documentId: frame.documentId });
        return response?.status === "ready" ? { frame, response } : null;
      } catch { return null; }
    }));
    const ready = replies.filter(Boolean);
    if (ready.length !== 1) return { status: "refused", reason: "no_suitable_focused_field" };
    const { frame, response } = ready[0];
    const result = await chrome.tabs.sendMessage(tabId, {
      ...probe, operation: "fill", token: response.token, value: request.value,
    }, { documentId: frame.documentId });
    if (result?.status === "filled" && result.field === request.field &&
        request.allowed_origins.includes(result.origin)) {
      return { status: "filled", field: request.field, origin: result.origin };
    }
    return { status: "refused", reason: "input_rejected" };
  }

  async function connect() {
    const self = await chrome.management.getSelf();
    if (self.installType !== "admin" || self.mayDisable) return;
    port = chrome.runtime.connectNative("dev.nyxid.machine_filler");
    port.onDisconnect.addListener(() => {
      void chrome.runtime.lastError;
      port = undefined;
      setTimeout(connect, retryMs);
      retryMs = Math.min(retryMs * 2, 30000);
    });
    port.onMessage.addListener(async request => {
      const currentPort = port;
      const nonce = typeof request?.nonce === "string" ? request.nonce : "";
      if (active) {
        currentPort.postMessage({ nonce, status: "refused", reason: "busy" });
        return;
      }
      active = true;
      retryMs = 500;
      let result;
      try {
        if (request?.operation === "browser") {
          if (!/^[0-9a-f-]{36}$/.test(nonce) || request.expires_at_ms < Date.now() || request.expires_at_ms > Date.now() + 30000 || seen.has(nonce)) {
            result = {status: "refused", reason: "expired_or_replayed"};
          } else {
            for (const [id, expiry] of seen) if (expiry <= Date.now()) seen.delete(id);
            if (seen.size >= 512) throw new Error("busy");
            seen.set(nonce, request.expires_at_ms);
            result = await NyxIdBrowserBackground.perform(request);
          }
        } else result = await fill(request);
      }
      catch { result = { status: "refused", reason: "browser_unavailable" }; }
      finally { if (request) request.value = ""; active = false; }
      try { currentPort.postMessage({ nonce, ...result }); }
      catch { /* A disconnected request is never replayed automatically. */ }
    });
    port.postMessage({ type: "hello", version: 1, extension_id: chrome.runtime.id });
  }
  void connect();
})();
