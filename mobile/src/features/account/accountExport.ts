/** One bounded file stream, bound to the admitted account for its lifetime. */
export async function shareAccountExport(deps: {
  available(): Promise<boolean>;
  request(): Promise<{
    response: { headers: { get(name: string): string | null }; body: { getReader(): { read(): Promise<{ done: boolean; value?: Uint8Array }>; cancel(): Promise<unknown>; releaseLock(): void } } | null };
    assertOwner(): Promise<void>;
  }>;
  create(): { uri: string; write(bytes: Uint8Array): void; close(): void; text(): Promise<string>; remove(): void };
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
    for (;;) {
      const chunk = await reader.read();
      await assertOwner();
      if (chunk.done) break;
      if (!chunk.value) continue;
      bytes += chunk.value.byteLength;
      if (bytes > 32 * 1024 * 1024) throw new Error("export_too_large");
      file.write(chunk.value);
    }
    const length = response.headers.get("Content-Length");
    if (length !== null && (!/^\d+$/.test(length) || bytes !== Number(length))) {
      throw new Error("export_incomplete");
    }
    file.close(); closed = true;
    // Also validate a complete JSON document when a proxy omits the length.
    // The response/file size is capped above before any write.
    const data: unknown = JSON.parse(await file.text());
    if (!data || typeof data !== "object" || !("format" in data) || data.format !== "nyxid.account-export") {
      throw new Error("export_invalid");
    }
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
