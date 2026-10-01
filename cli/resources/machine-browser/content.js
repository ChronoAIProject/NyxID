/* Runs only in Chrome's isolated world; never exports a page message bridge. */
(() => {
  const pending = new Map();
  const consumed = new Map();
  const pinned = new WeakSet();

  function prune(now) {
    for (const [nonce, item] of pending) if (item.expires <= now) pending.delete(nonce);
    for (const [nonce, expires] of consumed) if (expires <= now) consumed.delete(nonce);
  }

  function focusedInput(request) {
    if (!request.allowed_origins.includes(location.origin)) return "origin_mismatch";
    if (!document.hasFocus() || document.visibilityState !== "visible") return "not_focused";
    let element = document.activeElement;
    while (element?.shadowRoot?.activeElement) element = element.shadowRoot.activeElement;
    if (!(element instanceof HTMLInputElement) || !element.isConnected ||
        element.disabled || element.readOnly ||
        !NyxIdFillerPolicy.suitable(request.field, element.type)) return "wrong_field";
    if (!element.getClientRects().length) return "not_visible";
    return element;
  }

  function pinPassword(input) {
    if (pinned.has(input)) return;
    pinned.add(input);
    const observer = new MutationObserver(() => {
      if (input.type !== "password") input.type = "password";
    });
    observer.observe(input, { attributes: true, attributeFilter: ["type"] });
    const stop = () => {
      observer.disconnect();
      pinned.delete(input);
      input.form?.removeEventListener("submit", stop, true);
      window.removeEventListener("pagehide", stop, true);
    };
    input.form?.addEventListener("submit", stop, { capture: true, once: true });
    window.addEventListener("pagehide", stop, { capture: true, once: true });
  }

  // Also closes the interval between a type mutation and its observer callback.
  for (const type of ["copy", "cut"]) {
    document.addEventListener(type, event => {
      if (event.composedPath().some(element => pinned.has(element))) {
        event.preventDefault();
        event.stopImmediatePropagation();
      }
    }, true);
  }

  chrome.runtime.onMessage.addListener((request, sender, respond) => {
    if (sender.id !== chrome.runtime.id) return;
    const invalid = NyxIdFillerPolicy.validate(request);
    if (invalid) {
      respond({ status: "refused", reason: invalid });
      return;
    }
    prune(Date.now());
    if (consumed.has(request.nonce)) {
      respond({ status: "refused", reason: "replayed" });
      return;
    }
    const input = focusedInput(request);
    if (typeof input === "string") {
      respond({ status: "refused", reason: input });
      return;
    }
    if (request.operation === "probe") {
      if (pending.size >= 32 || consumed.size >= 512) {
        respond({ status: "refused", reason: "busy" });
        return;
      }
      const token = crypto.randomUUID();
      pending.set(request.nonce, { token, input, expires: request.expires_at_ms });
      respond({ status: "ready", token, origin: location.origin });
      return;
    }
    if (request.operation !== "fill") return;
    const selected = pending.get(request.nonce);
    pending.delete(request.nonce);
    consumed.set(request.nonce, request.expires_at_ms);
    if (!selected || selected.token !== request.token || selected.input !== input) {
      respond({ status: "refused", reason: "focus_changed" });
      return;
    }
    if (!NyxIdFillerPolicy.valueAllowed(request.value)) {
      respond({ status: "refused", reason: "invalid_value" });
      return;
    }
    try {
      if (request.field === "password") pinPassword(input);
      // The editing command uses the browser's normal text insertion machinery.
      // No value is assigned to a page-global variable or custom DOM attribute.
      input.select();
      const inserted = document.execCommand("insertText", false, request.value);
      const accepted = inserted && input.value === request.value;
      request.value = "";
      respond(accepted
        ? { status: "filled", field: request.field, origin: location.origin }
        : { status: "refused", reason: "input_rejected" });
    } catch {
      request.value = "";
      respond({ status: "refused", reason: "input_rejected" });
    }
  });
})();
