/**
 * One-shot recovery from a failed lazy-asset load.
 *
 * After a deploy, a tab running the previous build asks for chunk URLs that no
 * longer exist. A single reload picks up the current shell and fixes it, which
 * is why the recovery is silent: the user sees a refresh, not an error page.
 *
 * The bound is the interesting part. The previous implementation (#357) stored a
 * boolean and cleared it on every successful bootstrap — but bootstrap runs from
 * the entry chunk, which loads fine; it is the *route* chunk that fails. So the
 * guard was wiped before the failure it was meant to gate, and a permanently
 * missing chunk reloaded the tab forever instead of ever surfacing a message.
 *
 * Keying the guard on build identity fixes that without a clear step:
 *   - stored !== current build  ->  we have not reloaded on this build  ->  reload
 *   - stored === current build  ->  we reloaded and came back to the same broken
 *                                   build  ->  stop, let the caller show a message
 *
 * A deploy changes the build id, so the guard re-arms itself. Reloading is capped
 * at once per build, so it cannot spin.
 */

const STORAGE_KEY = "nyxid_chunk_reload";

/**
 * Message fragments emitted when a dynamic import or preload cannot be fetched
 * or is served with the wrong MIME type. Cross-browser: Chrome and Safari word
 * this differently, and Firefox differs in case as well as wording, so matching
 * is done case-insensitively against the normalized message.
 */
const ASSET_ERROR_PATTERNS = [
  // Chrome/Edge: dynamic import network failure or non-2xx
  "failed to fetch dynamically imported module",
  // Firefox: same condition, different wording
  "error loading dynamically imported module",
  // Safari
  "importing a module script failed",
  // Chrome, when a missing chunk falls through to an HTML error page
  "failed to load module script",
  // Vite's own CSS preload rejection (`__vitePreload`)
  "unable to preload css for",
  // Bundler-agnostic legacy wording
  "chunkloaderror",
  "loading chunk",
  "loading css chunk",
];

/** Pull a comparable string out of an unknown throwable. */
function messageOf(error: unknown): string {
  if (error instanceof Error) {
    return `${error.name} ${error.message}`;
  }
  if (typeof error === "string") {
    return error;
  }
  if (error && typeof error === "object" && "message" in error) {
    const { message } = error as { message: unknown };
    if (typeof message === "string") {
      return message;
    }
  }
  return "";
}

/**
 * True when the failure is "the browser could not load part of the app", as
 * opposed to a genuine render error. Accepts `unknown` because rejected promises
 * and DOM events can carry non-`Error` values.
 */
export function isAssetLoadError(error: unknown): boolean {
  const message = messageOf(error).toLowerCase();
  if (!message) {
    return false;
  }
  return ASSET_ERROR_PATTERNS.some((pattern) => message.includes(pattern));
}

// `sessionStorage` throws rather than no-ops when storage is partitioned or
// disabled (Safari private browsing, sandboxed iframes). Every access is guarded:
// an unguarded read in the old boundary ran inside `getDerivedStateFromError`,
// where a throw takes down the whole tree.
function readGuard(): string | null {
  try {
    return window.sessionStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeGuard(value: string): boolean {
  try {
    window.sessionStorage.setItem(STORAGE_KEY, value);
    return true;
  } catch {
    return false;
  }
}

function clearGuard(): void {
  try {
    window.sessionStorage.removeItem(STORAGE_KEY);
  } catch {
    // Nothing to do: without storage the guard was never set.
  }
}

export type RecoveryOutcome =
  /** A reload has been triggered; the caller should render nothing further. */
  | "reloading"
  /** Already reloaded on this build and still failing; show the user a message. */
  | "exhausted";

export interface RecoveryOptions {
  /** Overridable for tests; defaults to the build id injected by Vite. */
  readonly buildId?: string;
  /** Overridable for tests; defaults to a real page reload. */
  readonly reload?: () => void;
}

/**
 * Decide once, for this build, whether to reload. Idempotent: calling it again
 * after it has returned "reloading" (or from a second call site observing the
 * same failure) returns "exhausted" rather than reloading twice.
 */
export function recoverFromAssetError(
  options: RecoveryOptions = {},
): RecoveryOutcome {
  const {
    buildId = __BUILD_ID__,
    reload = () => window.location.reload(),
  } = options;

  if (readGuard() === buildId) {
    return "exhausted";
  }

  // Without a durable guard the reload could not be bounded, and an unbounded
  // reload is worse than an error message — so decline rather than risk a loop.
  if (!writeGuard(buildId)) {
    return "exhausted";
  }

  reload();
  return "reloading";
}

/**
 * Re-arm the guard for an explicit user retry. The automatic path is capped at
 * one reload per build; a person clicking "Try again" is asking for another.
 */
export function retryAfterAssetError(reload: () => void = () =>
  window.location.reload()): void {
  clearGuard();
  reload();
}
