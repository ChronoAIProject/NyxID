import { assistantHttp } from "./assistant-http";
import { subscribeAssistantIdentity } from "./identity";

const FILE_BYTES = 8 * 1024 * 1024;
const CACHE_BYTES = 32 * 1024 * 1024;
const MAX_ENTRIES = 64;
const IDLE_MS = 120_000;
interface Entry {
  readonly key: string;
  readonly controller: AbortController;
  readonly ready: Promise<string>;
  readonly reservedBytes: number;
  resolve: (url: string) => void;
  reject: (error: unknown) => void;
  refs: number;
  started: boolean;
  settled: boolean;
  url?: string;
  timer?: ReturnType<typeof setTimeout>;
}
const entries = new Map<string, Entry>();
const inFlight = new Set<Entry>();

function remove(entry: Entry) {
  if (entries.get(entry.key) !== entry) return;
  entries.delete(entry.key);
  clearTimeout(entry.timer);
  entry.controller.abort();
  if (entry.url) URL.revokeObjectURL(entry.url);
  entry.reject(new DOMException("Attachment released", "AbortError"));
}
async function fetchBlob(entry: Entry) {
  const response = await assistantHttp(entry.key, {
    signal: entry.controller.signal,
  });
  const declared = Number(response.headers.get("content-length"));
  if (declared > entry.reservedBytes)
    throw new Error("Generated file exceeds download limit");
  const reader = response.body?.getReader();
  if (!reader) throw new Error("Generated file body unavailable");
  const parts: Uint8Array<ArrayBuffer>[] = [];
  let size = 0;
  try {
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) break;
      size += chunk.value.byteLength;
      if (size > entry.reservedBytes)
        throw new Error("Generated file exceeds download limit");
      parts.push(chunk.value as Uint8Array<ArrayBuffer>);
    }
  } finally {
    await reader.cancel();
  }
  return new Blob(parts, {
    type: response.headers.get("content-type") ?? "application/octet-stream",
  });
}
function pump() {
  // Removed downloads keep their reservation until cancellation releases the body.
  const started = new Set([
    ...[...entries.values()].filter((entry) => entry.started),
    ...inFlight,
  ]);
  let resident = started.size;
  let bytes = [...started].reduce((sum, entry) => sum + entry.reservedBytes, 0);
  for (const entry of entries.values()) {
    if (entry.started || inFlight.size >= 2) continue;
    while (resident >= MAX_ENTRIES || bytes + entry.reservedBytes > CACHE_BYTES) {
      const unused = [...entries.values()].find(
        (candidate) =>
          candidate.started && candidate.settled && candidate.refs === 0,
      );
      if (!unused) return;
      remove(unused);
      resident -= 1;
      bytes -= unused.reservedBytes;
    }
    entry.started = true;
    resident += 1;
    bytes += entry.reservedBytes;
    inFlight.add(entry);
    void fetchBlob(entry)
      .then((blob) => {
        if (entries.get(entry.key) !== entry || entry.controller.signal.aborted)
          return;
        entry.url = URL.createObjectURL(blob);
        entry.resolve(entry.url);
      })
      .catch((error: unknown) => {
        entry.reject(error);
        remove(entry);
      })
      .finally(() => {
        entry.settled = true;
        inFlight.delete(entry);
        pump();
      });
  }
}
/** A lease pins a URL; unmount releases it without refetching on a quick remount. */
export function acquireGeneratedAttachment(
  endpoint: string,
  expiresAt?: string,
  size?: number,
) {
  let entry = entries.get(endpoint);
  if (!entry) {
    let resolve!: Entry["resolve"];
    let reject!: Entry["reject"];
    const ready = new Promise<string>((yes, no) => {
      resolve = yes;
      reject = no;
    });
    entry = {
      key: endpoint,
      reservedBytes:
        size !== undefined && Number.isSafeInteger(size) && size > 0 && size <= FILE_BYTES
          ? size
          : FILE_BYTES,
      controller: new AbortController(),
      ready,
      resolve,
      reject,
      refs: 0,
      started: false,
      settled: false,
    };
    entries.set(endpoint, entry);
  }
  entries.delete(endpoint);
  entries.set(endpoint, entry);
  clearTimeout(entry.timer);
  entry.refs += 1;
  const lease = entry;
  const expiry = Date.parse(expiresAt ?? "");
  if (Number.isFinite(expiry)) {
    lease.timer = setTimeout(
      () => {
        remove(lease);
        pump();
      },
      Math.min(Math.max(0, expiry - Date.now()), 2_147_483_647),
    );
  }
  pump();
  let released = false;
  return {
    ready: lease.ready,
    release() {
      if (released) return;
      released = true;
      lease.refs -= 1;
      if (lease.refs === 0) {
        clearTimeout(lease.timer);
        // Abort a fetch when nobody needs it. Completed files survive list remounts.
        if (!lease.settled) remove(lease);
        else
          lease.timer = setTimeout(
            () => {
              remove(lease);
              pump();
            },
            Math.min(
              IDLE_MS,
              Number.isFinite(expiry)
                ? Math.max(0, expiry - Date.now())
                : IDLE_MS,
            ),
          );
        pump();
      }
    },
  };
}
export function clearGeneratedAttachmentCache() {
  for (const entry of entries.values()) remove(entry);
}
subscribeAssistantIdentity(clearGeneratedAttachmentCache);
