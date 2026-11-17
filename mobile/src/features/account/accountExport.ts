/** Headers: server 60-second build budget plus 15 seconds for admission/network. */
export const ACCOUNT_EXPORT_HEADERS_TIMEOUT_MS = 75_000;
/** Body idle time begins only after headers; overall includes 120-second delivery. */
export const ACCOUNT_EXPORT_IDLE_TIMEOUT_MS = 30_000;
export const ACCOUNT_EXPORT_DEADLINE_MS = ACCOUNT_EXPORT_HEADERS_TIMEOUT_MS + 120_000;
/** Native cancellation can also hang after a failed read. */
export const ACCOUNT_EXPORT_CANCEL_TIMEOUT_MS = 1_000;

type ExportReader = {
  read(): Promise<{ done: boolean; value?: Uint8Array }>;
  cancel(): Promise<unknown>;
  releaseLock(): void;
};

function isJsonWhitespace(byte: number) {
  return byte === 0x20 || byte === 0x09 || byte === 0x0a || byte === 0x0d;
}

/** One bounded file stream, bound to the admitted account for its lifetime. */
export async function shareAccountExport(deps: {
  available(): Promise<boolean>;
  request(signal: AbortSignal): Promise<{
    response: { headers: { get(name: string): string | null }; body: { getReader(): ExportReader } | null };
    assertOwner(): Promise<void>;
  }>;
  create(): { uri: string; write(bytes: Uint8Array): void; close(): void; remove(): void };
  share(uri: string): Promise<void>;
}) {
  const controller = new AbortController();
  let headersTimer: ReturnType<typeof setTimeout> | undefined;
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let deadlineTimer: ReturnType<typeof setTimeout> | undefined;
  let timeoutError: Error | undefined;
  function clearWatchdogs() {
    if (headersTimer !== undefined) clearTimeout(headersTimer);
    headersTimer = undefined;
    if (idleTimer !== undefined) clearTimeout(idleTimer);
    if (deadlineTimer !== undefined) clearTimeout(deadlineTimer);
    idleTimer = undefined;
    deadlineTimer = undefined;
  }
  function expire(message: string) {
    timeoutError = new Error(message);
    clearWatchdogs();
    controller.abort(timeoutError);
  }
  function resetIdle() {
    if (idleTimer !== undefined) clearTimeout(idleTimer);
    idleTimer = setTimeout(() => expire("export_idle_timeout"), ACCOUNT_EXPORT_IDLE_TIMEOUT_MS);
  }
  async function waitFor<T>(operation: () => Promise<T>): Promise<T> {
    if (controller.signal.aborted) throw timeoutError ?? new Error("export_cancelled");
    let onAbort!: () => void;
    const interrupted = new Promise<never>((_, reject) => {
      onAbort = () => reject(timeoutError ?? new Error("export_cancelled"));
      controller.signal.addEventListener("abort", onAbort, { once: true });
    });
    try {
      // Race each wait: native fetch/read can ignore abort. Remove each
      // listener afterwards instead of accumulating handlers per chunk.
      return await Promise.race([interrupted, Promise.resolve().then(() => {
        if (controller.signal.aborted) throw timeoutError ?? new Error("export_cancelled");
        return operation();
      })]);
    } finally {
      controller.signal.removeEventListener("abort", onAbort);
    }
  }
  let reader: ExportReader | undefined;
  let file: ReturnType<typeof deps.create> | undefined;
  let closed = false;
  deadlineTimer = setTimeout(() => expire("export_deadline_exceeded"), ACCOUNT_EXPORT_DEADLINE_MS);
  try {
    if (!(await waitFor(() => deps.available()))) throw new Error("sharing_unavailable");
    headersTimer = setTimeout(() => expire("export_headers_timeout"), ACCOUNT_EXPORT_HEADERS_TIMEOUT_MS);
    const { response, assertOwner } = await waitFor(() => deps.request(controller.signal));
    clearTimeout(headersTimer);
    headersTimer = undefined;
    resetIdle();
    await waitFor(assertOwner);
    if (!response.body) throw new Error("export_stream_unavailable");
    const source = response.body.getReader();
    reader = source;
    file = deps.create();
    let bytes = 0;
    const encoding = response.headers.get("Content-Encoding");
    // Fetch exposes decoded bytes; a compressed wire length cannot verify them.
    const length = encoding !== null && encoding.trim().toLowerCase() !== "identity"
      ? null : response.headers.get("Content-Length");
    let firstByte: number | undefined;
    let lastByte: number | undefined;
    for (;;) {
      const chunk = await waitFor(() => source.read());
      if (!chunk.done && chunk.value?.byteLength) resetIdle();
      await waitFor(assertOwner);
      if (chunk.done) break;
      if (!chunk.value) continue;
      bytes += chunk.value.byteLength;
      if (bytes > 32 * 1024 * 1024) throw new Error("export_too_large");
      if (length === null) {
        // Keep only the outer non-whitespace bytes, never a body string or
        // retained chunks. A present length needs only the exact byte count.
        for (let i = 0; firstByte === undefined && i < chunk.value.length; i++) {
          const byte = chunk.value[i];
          if (byte !== undefined && !isJsonWhitespace(byte)) firstByte = byte;
        }
        for (let i = chunk.value.length - 1; i >= 0; i--) {
          const byte = chunk.value[i];
          if (byte !== undefined && !isJsonWhitespace(byte)) {
            lastByte = byte;
            break;
          }
        }
      }
      file.write(chunk.value);
    }
    if (length !== null && (!/^\d+$/.test(length) || bytes !== Number(length))) {
      throw new Error("export_incomplete");
    }
    // Reaching done without a read error plus these outer braces is the
    // bounded structural check when a proxy omits length or encodes the body.
    if (length === null && (firstByte !== 0x7b || lastByte !== 0x7d)) {
      throw new Error("export_incomplete");
    }
    file.close(); closed = true;
    await waitFor(assertOwner);
    // The share sheet is user-controlled; only transfer/validation is timed.
    clearWatchdogs();
    await deps.share(file.uri);
  } catch (error) {
    if (!controller.signal.aborted) controller.abort(error);
    throw error;
  } finally {
    clearWatchdogs();
    try {
      // Remove the file before waiting on possibly-stuck native cancellation.
      try { if (!closed) file?.close(); } finally { file?.remove(); }
    } finally {
      if (reader) {
        const source = reader;
        let cancelTimer: ReturnType<typeof setTimeout> | undefined;
        try {
          await Promise.race([
            Promise.resolve().then(() => source.cancel()),
            new Promise<void>((resolve) => {
              cancelTimer = setTimeout(resolve, ACCOUNT_EXPORT_CANCEL_TIMEOUT_MS);
            }),
          ]);
        } catch { /* Failed native cancellation cannot prevent cleanup. */ }
        finally {
          if (cancelTimer !== undefined) clearTimeout(cancelTimer);
          try { source.releaseLock(); } catch { /* A native read may still be pending. */ }
        }
      }
    }
  }
}

/** Mount-time recovery of files left by a killed app; no other cache is touched. */
export function cleanStaleAccountExports(files: readonly { name: string; delete(): void }[]) {
  for (const file of files) {
    if (/^nyxid-account-export-.*\.json$/.test(file.name)) {
      try { file.delete(); } catch { /* Retry the next time Privacy mounts. */ }
    }
  }
}
