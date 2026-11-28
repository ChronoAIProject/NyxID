import { assistantHttp } from "./assistant-http";
import { subscribeAssistantIdentity } from "./identity";

const FILE_BYTES = 8 * 1024 * 1024;
const CACHE_BYTES = 32 * 1024 * 1024;
const MAX_ENTRIES = 64;
const IDLE_MS = 120_000;
const MAX_TIMEOUT_MS = 2_147_483_647;
interface Entry {
  readonly key: string;
  readonly expiresAt?: number;
  readonly controller: AbortController;
  readonly ready: Promise<string>;
  readonly reservedBytes: number;
  resolve: (url: string) => void;
  reject: (error: unknown) => void;
  refs: number;
  started: boolean;
  settled: boolean;
  url?: string;
  idleUntil?: number;
  timer?: ReturnType<typeof setTimeout>;
}
const entries = new Map<string, Entry>();
const inFlight = new Set<Entry>();

function remove(entry: Entry) {
  if (entries.get(entry.key) !== entry) return;
  entries.delete(entry.key);
  clearTimeout(entry.timer);
  entry.timer = undefined;
  entry.controller.abort();
  if (entry.url) URL.revokeObjectURL(entry.url);
  entry.reject(new DOMException("Attachment released", "AbortError"));
}
function removalDeadline(entry: Entry) {
  return Math.min(
    entry.expiresAt ?? Infinity,
    entry.refs === 0 ? (entry.idleUntil ?? Infinity) : Infinity,
  );
}
function scheduleRemoval(entry: Entry) {
  clearTimeout(entry.timer);
  entry.timer = undefined;
  if (entries.get(entry.key) !== entry) return;
  const remaining = removalDeadline(entry) - Date.now();
  if (remaining <= 0) {
    remove(entry);
  } else if (Number.isFinite(remaining)) {
    // Browser timers cap at ~24.85 days; a wakeup must recheck the real deadline.
    entry.timer = setTimeout(
      () => {
        scheduleRemoval(entry);
        pump();
      },
      Math.min(remaining, MAX_TIMEOUT_MS),
    );
  }
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
  for (const entry of entries.values()) {
    if (removalDeadline(entry) <= Date.now()) remove(entry);
  }
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
        if (removalDeadline(entry) <= Date.now()) {
          remove(entry);
          return;
        }
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
  const parsedExpiry = Date.parse(expiresAt ?? "");
  const expiry = Number.isFinite(parsedExpiry) ? parsedExpiry : undefined;
  let entry = entries.get(endpoint);
  // Retire old URLs on a changed declaration; authorize the replacement by GET.
  if (entry && (entry.expiresAt !== expiry || removalDeadline(entry) <= Date.now())) {
    remove(entry);
    entry = undefined;
  }
  if (!entry) {
    let resolve!: Entry["resolve"];
    let reject!: Entry["reject"];
    const ready = new Promise<string>((yes, no) => {
      resolve = yes;
      reject = no;
    });
    entry = {
      key: endpoint,
      expiresAt: expiry,
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
  entry.refs += 1;
  entry.idleUntil = undefined;
  const lease = entry;
  scheduleRemoval(lease);
  pump();
  let released = false;
  return {
    ready: lease.ready,
    release() {
      if (released) return;
      released = true;
      lease.refs -= 1;
      if (entries.get(lease.key) !== lease) return;
      if (lease.refs === 0) {
        // Abort a fetch when nobody needs it. Completed files survive list remounts.
        if (!lease.settled) remove(lease);
        else {
          lease.idleUntil = Date.now() + IDLE_MS;
          scheduleRemoval(lease);
        }
        pump();
      }
    },
  };
}
export function clearGeneratedAttachmentCache() {
  for (const entry of entries.values()) remove(entry);
}
subscribeAssistantIdentity(clearGeneratedAttachmentCache);
