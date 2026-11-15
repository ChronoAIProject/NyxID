function isJsonWhitespace(byte: number) {
  return byte === 0x20 || byte === 0x09 || byte === 0x0a || byte === 0x0d;
}

/** One bounded file stream, bound to the admitted account for its lifetime. */
export async function shareAccountExport(deps: {
  available(): Promise<boolean>;
  request(): Promise<{
    response: { headers: { get(name: string): string | null }; body: { getReader(): { read(): Promise<{ done: boolean; value?: Uint8Array }>; cancel(): Promise<unknown>; releaseLock(): void } } | null };
    assertOwner(): Promise<void>;
  }>;
  create(): { uri: string; write(bytes: Uint8Array): void; close(): void; remove(): void };
  share(uri: string): Promise<void>;
}) {
  if (!(await deps.available())) throw new Error("sharing_unavailable");
  const { response, assertOwner } = await deps.request();
  await assertOwner();
  if (!response.body) throw new Error("export_stream_unavailable");
  const reader = response.body.getReader();
  let file: ReturnType<typeof deps.create> | undefined;
  let closed = false;
  try {
    file = deps.create();
    let bytes = 0;
    const length = response.headers.get("Content-Length");
    let firstByte: number | undefined;
    let lastByte: number | undefined;
    for (;;) {
      const chunk = await reader.read();
      await assertOwner();
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
    // bounded structural check when a proxy omits Content-Length.
    if (length === null && (firstByte !== 0x7b || lastByte !== 0x7d)) {
      throw new Error("export_incomplete");
    }
    file.close(); closed = true;
    await assertOwner();
    await deps.share(file.uri);
  } finally {
    try { await reader.cancel(); } catch { /* A closed stream needs no cancel. */ }
    reader.releaseLock();
    try { if (!closed) file?.close(); } finally { file?.remove(); }
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
