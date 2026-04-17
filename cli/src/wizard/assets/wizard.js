(() => {
  "use strict";

  const csrfMeta = document.querySelector('meta[name="wizard-csrf"]');
  const CSRF = csrfMeta ? csrfMeta.getAttribute("content") : "";

  const statusEl = document.getElementById("wizard-status");
  const doneBtn = document.getElementById("wizard-done");
  const cancelBtn = document.getElementById("wizard-cancel");
  const originEl = document.getElementById("wizard-origin");

  if (originEl) originEl.textContent = window.location.origin;

  function setStatus(msg, cls) {
    if (!statusEl) return;
    statusEl.textContent = msg;
    statusEl.className = "wizard-status" + (cls ? " " + cls : "");
  }

  function disableButtons() {
    if (doneBtn) doneBtn.disabled = true;
    if (cancelBtn) cancelBtn.disabled = true;
  }

  async function postLifecycle(path, body) {
    const res = await fetch(path, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-wizard-csrf": CSRF,
      },
      body: JSON.stringify(body || {}),
    });
    return res;
  }

  async function onDone() {
    disableButtons();
    setStatus("Signalling CLI…");
    try {
      const res = await postLifecycle("/api/proxy/complete", {
        milestone: "M1",
        result: "skeleton-ok",
      });
      if (!res.ok) {
        setStatus("CLI rejected the completion signal (HTTP " + res.status + ").", "error");
        return;
      }
      setStatus("Done. You can close this tab and return to your terminal.", "success");
    } catch (err) {
      setStatus("Couldn't reach the CLI: " + err.message, "error");
    }
  }

  async function onCancel() {
    disableButtons();
    setStatus("Cancelling…");
    try {
      await postLifecycle("/api/proxy/cancel", {});
      setStatus("Cancelled. You can close this tab.", "success");
    } catch (err) {
      setStatus("Couldn't reach the CLI: " + err.message, "error");
    }
  }

  // Best-effort cancel on tab close via sendBeacon. Browsers are
  // inconsistent about firing this (especially Chrome/Safari on ⌘W), so
  // we also run a heartbeat loop below — that's the robust detector.
  window.addEventListener("beforeunload", () => {
    try {
      const payload = new Blob([JSON.stringify({ reason: "unload" })], {
        type: "application/json",
      });
      navigator.sendBeacon("/api/proxy/cancel-unload", payload);
    } catch (_) { /* ignore */ }
  });

  // Heartbeat loop. Tick every 10 s while the page is visible. If the CLI
  // stops seeing heartbeats for ~22 s it will treat the tab as closed and
  // exit with "cancelled". No effort is made while the tab is hidden
  // (user switched apps) — visibilitychange resumes the loop on return.
  const HEARTBEAT_INTERVAL_MS = 10_000;
  let heartbeatTimer = null;
  async function sendHeartbeat() {
    try {
      await fetch("/api/proxy/heartbeat", {
        method: "POST",
        headers: { "content-type": "application/json", "x-wizard-csrf": CSRF },
        body: "{}",
        keepalive: true,
      });
    } catch (_) { /* page will be torn down anyway */ }
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
    if (document.hidden) stopHeartbeats();
    else startHeartbeats();
  });
  if (!document.hidden) startHeartbeats();

  if (doneBtn) doneBtn.addEventListener("click", onDone);
  if (cancelBtn) cancelBtn.addEventListener("click", onCancel);
})();
