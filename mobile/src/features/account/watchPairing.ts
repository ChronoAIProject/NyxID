import type {
  IssuedLoginCode,
  LoginCodeStatus,
} from "../../lib/api/loginCodeApi";
import type {
  WatchDelivery,
  WatchLink,
  WatchLinkStatus,
} from "../../lib/watch/watchLink";

/**
 * Setting up the NyxID Apple Watch app.
 *
 * The phone never shares its own session. It mints a one-time account login
 * code (the same `/auth/login-code` grant used to sign in other devices) and
 * hands only that code to the watch over WatchConnectivity. The watch redeems
 * it for its own session, which the person can revoke independently.
 *
 * Message format is versioned and mirrored in
 * `ios/NyxIDWatch/Core/PhoneMessage.swift`.
 */
export const WATCH_MESSAGE_VERSION = "1";

export function pairingMessage(
  code: IssuedLoginCode,
  apiBaseUrl: string,
  match: string,
  frontendUrl?: string,
): Record<string, string> {
  return {
    v: WATCH_MESSAGE_VERSION,
    kind: "pair",
    code: code.code,
    api_base_url: apiBaseUrl.replace(/\/+$/, ""),
    // The setup code the watch is showing: only that watch accepts this.
    match,
    ...(frontendUrl ? { frontend_url: frontendUrl.replace(/\/+$/, "") } : {}),
    expires_at: code.expires_at,
  };
}

/** "482913" → "482 913", as the watch shows it. */
export function displayWatchCode(code: string): string {
  return /^\d{6}$/.test(code) ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

/** The watch's current setup code, or null while it isn't showing one. */
export function watchSetupCode(status: WatchLinkStatus | null): string | null {
  const code = status?.watchCode ?? "";
  return /^\d{6}$/.test(code) ? code : null;
}

export function signOutMessage(): Record<string, string> {
  return { v: WATCH_MESSAGE_VERSION, kind: "sign_out" };
}

export type WatchAvailability =
  | "checking"
  | "unsupported"
  | "no_watch"
  | "not_installed"
  | "ready";

export function watchAvailability(
  status: WatchLinkStatus | null,
): WatchAvailability {
  if (!status) return "checking";
  if (!status.supported) return "unsupported";
  if (!status.activated || !status.paired) return "no_watch";
  if (!status.installed) return "not_installed";
  return "ready";
}

export type WatchPairingState =
  | { step: "idle" }
  | { step: "minting" }
  | { step: "sending" }
  | { step: "waiting"; delivery: WatchDelivery; requestId: string }
  | { step: "connected"; requestId: string }
  | { step: "failed"; message: string };

type Api = {
  mint(): Promise<IssuedLoginCode>;
  status(id: string): Promise<LoginCodeStatus>;
  cancel(id: string): Promise<void>;
};

type Options = {
  api: Api;
  link: Pick<WatchLink, "send" | "clearPending">;
  apiBaseUrl: () => string;
  /** The NyxID web app, so the watch can hand a chat to the iPhone. */
  frontendUrl?: () => string;
  notify: (state: WatchPairingState) => void;
  /**
   * Lets the caller handle an MFA step-up refusal on mint; resolves true
   * when it took over (the flow then returns to idle).
   */
  handleMintError?: (
    error: unknown,
    retry: () => Promise<void>,
  ) => Promise<boolean>;
  pollInterval?: number;
  now?: () => number;
};

/** Mint → send → follow the code until the watch redeems it. */
export class WatchPairingFlow {
  private generation = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private pendingCode: string | null = null;
  /** A code left queued in WatchConnectivity for the watch app's next run. */
  private queued = false;
  private state: WatchPairingState = { step: "idle" };

  constructor(private options: Options) {}

  get current(): WatchPairingState {
    return this.state;
  }

  /** Signs in the watch showing `match` (its setup code). */
  async start(match: string): Promise<void> {
    const { step } = this.state;
    if (step === "minting" || step === "sending" || step === "waiting") return;
    const generation = ++this.generation;
    this.clearTimer();
    this.emit({ step: "minting" });
    let code: IssuedLoginCode;
    try {
      code = await this.options.api.mint();
    } catch (error) {
      if (generation !== this.generation) return;
      if (
        this.options.handleMintError &&
        (await this.options.handleMintError(error, () => this.restart(match)))
      ) {
        if (generation === this.generation) this.emit({ step: "idle" });
        return;
      }
      this.emit({ step: "failed", message: mintFailure(error) });
      return;
    }
    if (generation !== this.generation) {
      void this.options.api.cancel(code.request_id).catch(() => {});
      return;
    }
    this.pendingCode = code.request_id;
    this.emit({ step: "sending" });
    let delivery: WatchDelivery;
    try {
      delivery = await this.options.link.send(
        pairingMessage(
          code,
          this.options.apiBaseUrl(),
          match,
          this.options.frontendUrl?.(),
        ),
      );
    } catch (error) {
      void this.cancelPending();
      if (generation === this.generation)
        this.emit({ step: "failed", message: sendFailure(error) });
      return;
    }
    this.queued = delivery === "queued";
    if (generation !== this.generation) {
      this.releaseQueued();
      return;
    }
    this.emit({ step: "waiting", delivery, requestId: code.request_id });
    this.schedule(generation, code.request_id, 0);
  }

  /** Stops following; an unredeemed code is cancelled so it can't be used. */
  stop(): void {
    this.generation++;
    this.clearTimer();
    void this.cancelPending();
    this.releaseQueued();
    if (this.state.step !== "connected") this.emit({ step: "idle" });
  }

  private async restart(match: string) {
    if (this.state.step !== "idle" && this.state.step !== "failed") return;
    await this.start(match);
  }

  private schedule(generation: number, requestId: string, delay: number) {
    this.clearTimer();
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.poll(generation, requestId);
    }, delay);
  }

  private async poll(generation: number, requestId: string) {
    let status: LoginCodeStatus | null = null;
    try {
      status = await this.options.api.status(requestId);
    } catch {
      // Transient: keep following until the code's own expiry.
    }
    if (generation !== this.generation) return;
    if (status?.status === "redeemed") {
      this.pendingCode = null;
      this.releaseQueued();
      this.emit({ step: "connected", requestId });
      return;
    }
    if (status && status.status !== "pending") {
      this.pendingCode = null;
      this.releaseQueued();
      this.emit({
        step: "failed",
        message:
          status.status === "expired"
            ? "The setup code expired before your watch used it. Open NyxID on your watch and try again."
            : "Setup was cancelled. Try again.",
      });
      return;
    }
    const expiresAt = status ? Date.parse(status.expires_at) : NaN;
    const now = (this.options.now ?? Date.now)();
    if (Number.isFinite(expiresAt) && now > expiresAt + 5_000) {
      void this.cancelPending();
      this.releaseQueued();
      this.emit({
        step: "failed",
        message:
          "The setup code expired before your watch used it. Open NyxID on your watch and try again.",
      });
      return;
    }
    this.schedule(generation, requestId, this.options.pollInterval ?? 2_000);
  }

  private async cancelPending() {
    const id = this.pendingCode;
    this.pendingCode = null;
    if (id) await this.options.api.cancel(id).catch(() => {});
  }

  /** Don't leave a finished code waiting in WatchConnectivity. */
  private releaseQueued() {
    if (!this.queued) return;
    this.queued = false;
    void this.options.link.clearPending().catch(() => {});
  }

  private clearTimer() {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  private emit(state: WatchPairingState) {
    this.state = state;
    this.options.notify(state);
  }
}

function errorCode(error: unknown): string | undefined {
  if (typeof error !== "object" || error === null) return undefined;
  const code = (error as { code?: unknown }).code;
  return typeof code === "string" ? code : undefined;
}

export function sendFailure(error: unknown): string {
  switch (errorCode(error)) {
    case "ERR_WATCH_NOT_PAIRED":
      return "No Apple Watch is paired with this iPhone.";
    case "ERR_WATCH_APP_NOT_INSTALLED":
      return "Install NyxID on your Apple Watch from the Watch app, then try again.";
    case "ERR_WATCH_UNSUPPORTED":
      return "This device can't pair with an Apple Watch.";
    default:
      return "Couldn't reach your watch. Open NyxID on your watch and try again.";
  }
}

export function mintFailure(error: unknown): string {
  const key =
    typeof error === "object" && error !== null
      ? (error as { errorKey?: unknown }).errorKey
      : undefined;
  if (key === "login_code_rate_limited")
    return "Too many setup attempts. Wait a few minutes and try again.";
  if (error instanceof Error && error.message) return error.message;
  return "Couldn't start setup. Try again.";
}
