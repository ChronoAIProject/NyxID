import {
  voicePreferencesSchema,
  voiceSessionSchema,
  voiceSnapshotSchema,
  voiceStartedSchema,
  voiceStartFailureSchema,
  type VoiceControlRequest,
  type VoicePreferences,
  type VoiceProtocol,
  type VoiceSession,
  type VoiceSnapshot,
  type VoiceStartRequest,
} from "./voiceContracts";
import { ApiError } from "../../../lib/api/ApiError";
import { discoverActiveVoiceCall } from "./voiceActiveCall";
import { withVoiceRequestDeadline } from "./voiceRequestDeadline";
import {
  voiceStartError,
  voiceStartRecovery,
  type VoiceStartRecovery,
  type VoiceStartStep,
} from "./voiceStartError";

export type VoiceMediaEvent =
  | { type: "provider-ready" }
  | { type: "playback-blocked" }
  | { type: "pcm"; bytes: ArrayBuffer }
  /** Mic and playback loudness, 0..1, about 20 Hz while either changes. */
  | { type: "level"; input: number; output: number }
  | { type: "error"; message: string };

/** Ephemeral device media only; this adapter never receives account credentials. */
export interface VoiceMediaAdapter {
  prepare(protocol: VoiceProtocol, signal: AbortSignal): Promise<string>;
  answer(sdp: string): Promise<void>;
  setInput(enabled: boolean): void;
  hold(pressed: boolean): void;
  muteSpeaker(muted: boolean): void;
  resume(): Promise<void>;
  enqueue(bytes: ArrayBuffer, endMs: number): void;
  flush(): void;
  playback(): { playback_ms: number; playback_audible: boolean };
  /** Transfer quiet native transport to this call's bounded End cleanup owner. */
  beginEndDrain?(): void;
  close(): void;
  subscribe(listener: (event: VoiceMediaEvent) => void): () => void;
}

export interface VoiceControlSocket {
  readyState: number;
  bufferedAmount: number;
  binaryType: string;
  onopen?: (() => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose: (() => void) | null;
  onerror: (() => void) | null;
  send(data: string | ArrayBuffer): void;
  close(): void;
}

export interface AssistantVoiceApi {
  start(
    conversationId: string,
    input: VoiceStartRequest,
    signal?: AbortSignal,
  ): Promise<unknown>;
  active(conversationId: string, signal?: AbortSignal): Promise<unknown>;
  accountActive?(
    conversationId: string,
    signal?: AbortSignal,
  ): Promise<unknown>;
  control(
    conversationId: string,
    sessionId: string,
    input: VoiceControlRequest,
    signal?: AbortSignal,
  ): Promise<unknown>;
  cancelRequest(
    conversationId: string,
    requestId: string,
    signal?: AbortSignal,
  ): Promise<unknown>;
}

export interface VoiceClientClock {
  setInterval(callback: () => void, ms: number): unknown;
  clearInterval(handle: unknown): void;
  setTimeout(callback: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface AssistantVoiceClientOptions {
  conversationId: string;
  media: VoiceMediaAdapter;
  api: AssistantVoiceApi;
  createSocket(path: string, signal: AbortSignal): Promise<VoiceControlSocket>;
  onSnapshot(snapshot: VoiceSnapshot): void;
  /** Provider and authoritative controls are ready, independently of mic mode. */
  onReady?(): void;
  /** Mirrors the capture gate applied to the media adapter, independently of readiness. */
  onInputChanged?(enabled: boolean): void;
  onError(message: string, recovery?: VoiceStartRecovery): void;
  onClosed?(): void;
  /** Durable cleanup ownership; may settle after this view has unmounted. */
  onCleanup?(
    session: VoiceSession | null,
    unknown: boolean,
    conversationId?: string,
  ): void;
  onConflict?(
    session: VoiceSession | null,
    conversationId?: string,
    unknown?: boolean,
  ): void;
  randomId?(): string;
  clock?: VoiceClientClock;
}

const defaultClock: VoiceClientClock = {
  setInterval: (callback, ms) => setInterval(callback, ms),
  clearInterval: (handle) =>
    clearInterval(handle as ReturnType<typeof setInterval>),
  setTimeout: (callback, ms) => setTimeout(callback, ms),
  clearTimeout: (handle) =>
    clearTimeout(handle as ReturnType<typeof setTimeout>),
};

const simulatorMicrophoneMessage =
  "A test microphone is active. Select a real microphone and try again.";

/** One explicit call attempt. It never retries a POST or silently restarts audio. */
export class AssistantVoiceClient {
  private readonly abort = new AbortController();
  private readonly clock: VoiceClientClock;
  private readonly randomId: () => string;
  private readonly unsubscribe: () => void;
  private closed = false;
  private disposed = false;
  private onDrainSnapshot?: (snapshot: VoiceSnapshot) => void;
  private attempted = false;
  private started = false;
  private ready = false;
  private readyNotified = false;
  private held = false;
  private inputEnabled = false;
  private speakerMuted = false;
  private revision = 0;
  private protocol: VoiceProtocol = "openai_live";
  private preferences?: VoicePreferences;
  private session?: VoiceSession;
  private snapshot?: VoiceSnapshot;
  private socket?: VoiceControlSocket;
  private heartbeat?: unknown;
  private readinessTimeout?: unknown;
  private pendingAudio?: { endMs: number; discard: boolean };
  private controlPending = false;
  private readonly endingSessions = new Set<string>();
  private endPromise?: Promise<void>;
  private outstandingEnd?: VoiceSession;
  private startInFlight = false;

  constructor(private readonly options: AssistantVoiceClientOptions) {
    this.clock = options.clock ?? defaultClock;
    this.randomId = options.randomId ?? (() => crypto.randomUUID());
    this.unsubscribe = options.media.subscribe((event) => this.onMedia(event));
    options.media.setInput(false);
  }

  get isClosed() {
    return this.closed;
  }

  get pendingEnd() {
    return this.outstandingEnd;
  }

  get pendingStart() {
    return this.startInFlight;
  }

  private path() {
    return `/assistant/nyxagent/conversations/${this.options.conversationId}/voice-sessions`;
  }

  async start(
    preferences: VoicePreferences,
    protocol: VoiceProtocol = "openai_live",
  ): Promise<void> {
    if (this.attempted || this.closed) return;
    this.attempted = true;
    this.protocol = protocol;
    let requested = false;
    let step: VoiceStartStep = "response";
    try {
      this.preferences = voicePreferencesSchema.parse(preferences);
      if (
        protocol === "xai_realtime" &&
        preferences.input_mode !== "push_to_talk"
      ) {
        this.fail("Grok voice requires Hold to talk and headphones.");
        return;
      }
      step = "microphone";
      const sdp = await this.options.media.prepare(protocol, this.abort.signal);
      if (this.closed) {
        this.options.media.close();
        return;
      }
      if (protocol === "openai_live" && !sdp.trim()) {
        this.fail("Voice media could not be prepared. Please try again.");
        return;
      }
      requested = true;
      this.startInFlight = true;
      step = "server";
      const raw = await this.options.api.start(
        this.options.conversationId,
        {
          client_request_id: this.randomId(),
          preferences: this.preferences,
          sdp_offer: protocol === "xai_realtime" ? "" : sdp,
        },
        this.abort.signal,
      );
      this.startInFlight = false;
      step = "response";
      if (raw && typeof raw === "object" && "session" in raw) {
        const ownSession = voiceSessionSchema.safeParse(raw.session);
        if (ownSession.success) this.session = ownSession.data;
      }
      const result = voiceStartedSchema.parse(raw);
      this.session = result.session;
      if (this.closed) {
        await this.endRemote(result.session);
        return;
      }
      if (protocol === "openai_live") {
        if (!result.sdp_answer.trim()) {
          this.fail(voiceStartError(undefined, "response"));
          return;
        }
        step = "webrtc";
        await this.options.media.answer(result.sdp_answer);
      } else this.started = true;
      if (this.closed) return;
      step = "transport";
      await this.openControls();
    } catch (error) {
      this.startInFlight = false;
      // A cancelled attempt can still return an owned session with a malformed
      // answer. Its validated identity remains enough to end that call, even
      // though the closed client must not publish errors or observe another one.
      if (this.closed && this.session) {
        await this.endRemote(this.session);
        return;
      }
      // A lost response does not establish whether the POST committed. Observe once;
      // never repeat creation, or end an unrelated pre-existing session on a 409.
      const active =
        requested && !this.session ? await this.observeUncertainStart() : null;
      if (!this.closed) {
        const conflict =
          typeof error === "object" &&
          error !== null &&
          (("statusCode" in error && error.statusCode === 409) ||
            ("status" in error && error.status === 409));
        this.fail(
          conflict
            ? active
              ? active.session.end_requested ||
                active.session.state === "closing"
                ? "The previous voice call is finishing. You can continue with text."
                : "Another voice call is active. End it before starting a new call."
              : "Voice state changed before the call could start. Check call status and try again."
            : step === "microphone" &&
                error instanceof Error &&
                error.message === simulatorMicrophoneMessage
              ? simulatorMicrophoneMessage
              : voiceStartError(error, step),
          conflict ? undefined : voiceStartRecovery(error, step),
        );
      }
    } finally {
      this.startInFlight = false;
    }
  }

  private async observeUncertainStart() {
    const controller = new AbortController();
    const interrupted = () => controller.abort();
    // A late POST may settle after local End. Its bounded metadata recovery
    // still belongs to onCleanup, while an in-view lookup follows local cancel.
    if (!this.closed)
      this.abort.signal.addEventListener("abort", interrupted, { once: true });
    try {
      const active = await withVoiceRequestDeadline(controller, () =>
        discoverActiveVoiceCall(
          this.options.api,
          this.options.conversationId,
          controller.signal,
        ),
      );
      if (!this.closed)
        this.options.onConflict?.(
          active?.session ?? null,
          active?.conversationId,
        );
      else
        this.options.onCleanup?.(
          active?.session ?? null,
          false,
          active?.conversationId,
        );
      return active;
    } catch {
      if (!this.closed) this.options.onConflict?.(null, undefined, true);
      else this.options.onCleanup?.(null, true);
      return null;
    } finally {
      this.abort.signal.removeEventListener("abort", interrupted);
      controller.abort();
    }
  }

  private async openControls() {
    if (!this.session || this.closed) return;
    const socket = await this.options.createSocket(
      `${this.path()}/${this.session.id}/stream`,
      this.abort.signal,
    );
    if (this.closed) {
      socket.close();
      return;
    }
    this.socket = socket;
    socket.binaryType = "arraybuffer";
    socket.onopen = () => {
      if (this.closed || this.socket !== socket) return;
      this.updateInput();
      this.notifyReady();
    };
    socket.onmessage = ({ data }) => {
      if (this.socket !== socket) return;
      if (this.closed) {
        // Only authoritative snapshots can settle an explicit End. Audio and
        // provider readiness never regain authority during this quiet drain.
        if (this.onDrainSnapshot && typeof data === "string") {
          try {
            this.onDrainSnapshot(voiceSnapshotSchema.parse(JSON.parse(data)));
          } catch {
            /* Keep the last verified cleanup target until the deadline. */
          }
        }
        return;
      }
      this.onMessage(data);
    };
    const disconnected = () => {
      if (this.closed || this.socket !== socket) return;
      this.fail("Voice connection lost. Start a new call to reconnect.");
    };
    socket.onclose = disconnected;
    socket.onerror = disconnected;
    this.readinessTimeout = this.clock.setTimeout(() => {
      if (!this.closed)
        this.fail("Voice connection timed out. Start a new call to reconnect.");
    }, 15_000);
    this.heartbeat = this.clock.setInterval(() => this.sendHeartbeat(), 1000);
  }

  private onMedia(event: VoiceMediaEvent) {
    if (this.closed) return;
    if (event.type === "provider-ready") {
      this.started = true;
      this.updateInput();
      this.notifyReady();
    } else if (event.type === "playback-blocked") {
      this.options.onError("Tap Resume audio to hear the assistant.");
    } else if (event.type === "error") {
      // Media exception strings may include device/provider details. Keep UI bounded.
      this.fail(
        event.message === simulatorMicrophoneMessage
          ? simulatorMicrophoneMessage
          : "Voice media stopped. Check microphone permission and start a new call.",
      );
    } else if (
      event.type === "pcm" &&
      this.protocol === "xai_realtime" &&
      this.inputEnabled
    ) {
      const bytes = event.bytes;
      if (
        !bytes.byteLength ||
        bytes.byteLength > 1920 ||
        bytes.byteLength % 2
      ) {
        this.fail("Voice audio could not be verified. Start a new call.");
        return;
      }
      if ((this.socket?.bufferedAmount ?? 0) > 96_000) {
        this.fail(
          "Voice connection is too slow. Start a new call to reconnect.",
        );
        return;
      }
      this.send(bytes);
    }
  }

  private onMessage(data: unknown) {
    try {
      if (data instanceof ArrayBuffer) {
        const frame = this.pendingAudio;
        this.pendingAudio = undefined;
        if (
          this.protocol !== "xai_realtime" ||
          !frame ||
          !data.byteLength ||
          data.byteLength > 1920 ||
          data.byteLength % 2
        )
          throw new Error("Invalid PCM frame");
        if (!frame.discard) this.options.media.enqueue(data, frame.endMs);
        return;
      }
      if (typeof data !== "string" || data.length > 262_144)
        throw new Error("Invalid voice event");
      const event: unknown = JSON.parse(data);
      if (!event || typeof event !== "object" || !("type" in event))
        throw new Error("Invalid voice event");
      if (event.type === "audio" || event.type === "audio_flush") {
        if (this.protocol !== "xai_realtime" || !("generation" in event))
          throw new Error("Unexpected voice audio");
        if (event.type === "audio_flush") {
          if (event.generation === this.session?.generation) {
            this.options.media.flush();
            this.pendingAudio = undefined;
          }
          return;
        }
        if (
          this.pendingAudio ||
          !("end_ms" in event) ||
          !Number.isSafeInteger(event.end_ms) ||
          (event.end_ms as number) < 0 ||
          (event.end_ms as number) > 1_830_000
        )
          throw new Error("Invalid playback watermark");
        this.pendingAudio = {
          endMs: event.end_ms as number,
          discard: event.generation !== this.session?.generation,
        };
        return;
      }
      if (event.type === "start_failed") {
        const failure = voiceStartFailureSchema.parse(event).error;
        const error = new ApiError({
          statusCode: 503,
          errorCode: failure.error_code,
          errorKey: failure.error,
          message: failure.message,
          details: failure.details,
        });
        this.fail(
          voiceStartError(error, "server"),
          voiceStartRecovery(error, "server"),
        );
        return;
      }
      const snapshot = voiceSnapshotSchema.parse(event);
      if (
        snapshot.session.id !== this.session?.id ||
        snapshot.session.generation < this.session.generation ||
        (snapshot.session.generation === this.session.generation &&
          snapshot.session.control_revision < this.session.control_revision)
      )
        return;
      const generationChanged =
        snapshot.session.generation !== this.session.generation;
      if (generationChanged) {
        this.pendingAudio = undefined;
        this.options.media.flush();
      }
      if (generationChanged || this.inputBlocked(snapshot.session))
        this.releaseHeldInput();
      this.session = snapshot.session;
      this.snapshot = snapshot;
      this.ready = true;
      this.updateInput();
      this.options.onSnapshot(snapshot);
      this.notifyReady();
      if (
        snapshot.session.state === "closed" ||
        snapshot.session.state === "failed"
      ) {
        if (snapshot.session.end_reason === "server_update")
          this.options.onError(
            "The call ended during a server update. Start a new call.",
          );
        this.dispose();
      }
    } catch {
      this.fail("Voice state could not be verified. Start a new call.");
    }
  }

  private inputBlocked(session: VoiceSession | undefined) {
    return (
      !session ||
      session.state !== "active" ||
      session.end_requested ||
      session.muted ||
      session.input_muted
    );
  }

  private notifyReady() {
    if (
      this.closed ||
      this.readyNotified ||
      !this.started ||
      !this.ready ||
      this.session?.state !== "active" ||
      this.session.end_requested ||
      this.socket?.readyState !== 1
    )
      return;
    this.readyNotified = true;
    this.clearReadinessTimeout();
    this.options.onReady?.();
  }

  /** Revoked input permission cannot keep a previous physical press latched. */
  private releaseHeldInput() {
    if (!this.held) return;
    this.held = false;
    this.applyInput(false);
    this.options.media.hold(false);
  }

  private applyInput(enabled: boolean) {
    const previous = this.inputEnabled;
    this.inputEnabled = enabled;
    this.options.media.setInput(enabled);
    // Media can synchronously fail and close while applying its gate. Never
    // report the older enable decision after that cleanup has disabled input.
    if (this.inputEnabled === enabled && previous !== enabled)
      this.options.onInputChanged?.(enabled);
  }

  private updateInput() {
    this.applyInput(
      Boolean(
        !this.closed &&
        this.started &&
        this.ready &&
        this.socket?.readyState === 1 &&
        !this.inputBlocked(this.session) &&
        !this.controlPending &&
        (this.preferences?.input_mode === "automatic" || this.held),
      ),
    );
  }

  private send(data: string | ArrayBuffer) {
    if (this.closed) return false;
    if (this.socket?.readyState !== 1) {
      this.fail("Voice connection lost. Start a new call to reconnect.");
      return false;
    }
    try {
      this.socket.send(data);
      return true;
    } catch {
      this.fail("Voice connection lost. Start a new call to reconnect.");
      return false;
    }
  }

  private sendHeartbeat() {
    if (!this.ready || !this.session || this.closed) return;
    const playback = this.options.media.playback();
    if (
      !Number.isSafeInteger(playback.playback_ms) ||
      playback.playback_ms < 0 ||
      playback.playback_ms > 1_830_000 ||
      typeof playback.playback_audible !== "boolean"
    ) {
      this.fail("Voice playback could not be verified. Start a new call.");
      return;
    }
    this.send(
      JSON.stringify({
        type: "heartbeat",
        generation: this.session.generation,
        revision: ++this.revision,
        playback_ms: playback.playback_ms,
        playback_audible: !this.speakerMuted && playback.playback_audible,
      }),
    );
  }

  hold(pressed: boolean): boolean {
    if (this.closed) return false;
    if (this.held === pressed) return this.held;
    if (
      pressed &&
      (!this.ready ||
        !this.started ||
        this.socket?.readyState !== 1 ||
        this.inputBlocked(this.session) ||
        this.controlPending)
    )
      return false;
    if (!pressed) this.releaseHeldInput();
    if (this.protocol === "xai_realtime") {
      if (
        !this.send(
          JSON.stringify({ type: pressed ? "ptt_begin" : "ptt_commit" }),
        )
      )
        return false;
    }
    this.held = pressed;
    this.options.media.hold(pressed);
    this.updateInput();
    return this.held;
  }

  /**
   * Switch Hands-free / Hold to talk during a call. OpenAI listens for turns
   * itself, so the mode only decides whether the microphone stays open; Grok
   * runs on explicit hold events and stays Hold to talk.
   */
  setInputMode(mode: VoicePreferences["input_mode"]): boolean {
    if (this.closed || !this.preferences) return false;
    if (this.protocol === "xai_realtime" && mode !== "push_to_talk")
      return false;
    if (this.preferences.input_mode === mode) return true;
    if (this.held) this.hold(false);
    this.preferences = { ...this.preferences, input_mode: mode };
    this.updateInput();
    return true;
  }

  muteSpeaker(muted: boolean) {
    if (this.closed) return;
    this.speakerMuted = muted;
    this.options.media.muteSpeaker(muted);
  }

  async resumeAudio() {
    if (this.closed) return;
    this.options.media.muteSpeaker(this.speakerMuted);
    try {
      await this.options.media.resume();
    } catch {
      if (!this.closed)
        this.options.onError("Tap Resume audio to hear the assistant.");
    }
  }

  async mute(muted: boolean) {
    if (!this.session || this.closed || this.controlPending) return;
    const session = this.session;
    this.hold(false);
    this.controlPending = true;
    this.updateInput();
    try {
      const next = voiceSessionSchema.parse(
        await this.options.api.control(
          this.options.conversationId,
          session.id,
          {
            command_id: this.randomId(),
            expected_revision: session.control_revision,
            action: muted ? "mute" : "unmute",
          },
          this.abort.signal,
        ),
      );
      if (this.closed) return;
      if (next.id !== session.id) throw new Error("Voice session changed");
      if (
        next.generation > this.session!.generation ||
        (next.generation === this.session!.generation &&
          next.control_revision >= this.session!.control_revision)
      )
        this.session = next;
    } catch {
      if (!this.closed)
        this.fail(
          "Voice controls changed. End this call and start a new call.",
        );
    } finally {
      this.controlPending = false;
      if (!this.closed) this.updateInput();
    }
  }

  async stopTask(id: string) {
    if (this.closed) return;
    try {
      await this.options.api.cancelRequest(
        this.options.conversationId,
        id,
        this.abort.signal,
      );
    } catch {
      if (!this.closed)
        this.options.onError(
          "The task could not be stopped. Try again from the chat.",
        );
    }
  }

  end(): Promise<void> {
    return this.endCall(true);
  }

  private endCall(graceful: boolean): Promise<void> {
    if (this.endPromise) return this.endPromise;
    if (this.closed) return Promise.resolve();
    const session = this.session;
    this.outstandingEnd = session;
    if (!session && this.startInFlight) this.options.onCleanup?.(null, true);
    const drain =
      graceful &&
      !!session &&
      this.protocol === "openai_live" &&
      this.socket?.readyState === 1;
    if (drain) {
      // Lock every input/output path synchronously, keeping only transport
      // alive so NyxID can obtain the provider's final close acknowledgement.
      this.closed = true;
      this.ready = false;
      this.clearReadinessTimeout();
      if (this.heartbeat !== undefined)
        this.clock.clearInterval(this.heartbeat);
      this.heartbeat = undefined;
      this.releaseHeldInput();
      this.applyInput(false);
      this.speakerMuted = true;
      this.options.media.muteSpeaker(true);
      this.options.media.flush();
      this.pendingAudio = undefined;
      this.options.media.beginEndDrain?.();
    } else this.dispose();
    this.endPromise = session
      ? this.endRemote(session, drain)
      : Promise.resolve();
    return this.endPromise;
  }

  private async endRemote(session: VoiceSession, drain = false) {
    if (this.endingSessions.has(session.id)) return;
    this.endingSessions.add(session.id);
    this.outstandingEnd = session;
    this.options.onCleanup?.(session, false, this.options.conversationId);
    const controller = new AbortController();
    let accepting = true;
    let terminalAccepted = false;
    let complete!: () => void;
    const finished = new Promise<void>((resolve) => {
      complete = resolve;
    });
    const accept = (next: VoiceSession, snapshot?: VoiceSnapshot) => {
      if (!accepting || terminalAccepted) return false;
      const observed = this.outstandingEnd ?? this.session ?? session;
      if (
        next.id !== session.id ||
        next.generation < observed.generation ||
        (next.generation === observed.generation &&
          next.control_revision < observed.control_revision)
      )
        return false;
      const terminal = next.state === "closed" || next.state === "failed";
      this.session = next;
      if (snapshot) this.snapshot = snapshot;
      this.outstandingEnd = terminal ? undefined : next;
      if (terminal) {
        terminalAccepted = true;
        this.options.onSnapshot({
          type: "snapshot",
          ...this.snapshot,
          session: next,
          captions: this.snapshot?.captions ?? [],
          tasks: this.snapshot?.tasks ?? [],
        });
        complete();
      }
      return true;
    };
    if (drain)
      this.onDrainSnapshot = (snapshot) => {
        accept(snapshot.session, snapshot);
      };
    // Absolute budget includes the POST itself. Abort is paired with a race so
    // even an adapter that ignores cancellation cannot strand local resources.
    const deadline = this.clock.setTimeout(() => {
      accepting = false;
      controller.abort();
      complete();
    }, 7_000);
    void (async () => {
      try {
        const next = voiceSessionSchema.parse(
          await this.options.api.control(
            this.options.conversationId,
            session.id,
            {
              command_id: this.randomId(),
              expected_revision: session.control_revision,
              action: "end",
            },
            controller.signal,
          ),
        );
        accept(next);
      } catch {
        // Retain the verified identity. A draining control socket can still
        // prove termination after a lost or invalid HTTP response.
      } finally {
        if (!drain) complete();
      }
    })();
    await finished;
    accepting = false;
    this.onDrainSnapshot = undefined;
    this.clock.clearTimeout(deadline);
    controller.abort();
    if (drain) this.dispose();
    this.options.onConflict?.(
      this.outstandingEnd ?? null,
      this.outstandingEnd ? this.options.conversationId : undefined,
    );
    this.options.onCleanup?.(
      this.outstandingEnd ?? null,
      false,
      this.options.conversationId,
    );
  }

  private fail(message: string, recovery?: VoiceStartRecovery) {
    if (this.closed) return;
    this.options.onError(message, recovery);
    // Device/transport faults require immediate local teardown. A human End
    // preserves quiet transport briefly; a failed transport cannot drain it.
    void this.endCall(false);
  }

  private clearReadinessTimeout() {
    if (this.readinessTimeout !== undefined)
      this.clock.clearTimeout(this.readinessTimeout);
    this.readinessTimeout = undefined;
  }

  private dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.closed = true;
    this.ready = false;
    this.releaseHeldInput();
    this.abort.abort();
    this.clearReadinessTimeout();
    if (this.heartbeat !== undefined) this.clock.clearInterval(this.heartbeat);
    this.heartbeat = undefined;
    this.pendingAudio = undefined;
    this.unsubscribe();
    this.applyInput(false);
    this.options.media.close();
    if (this.socket) {
      this.socket.onopen = null;
      this.socket.onmessage = null;
      this.socket.onclose = null;
      this.socket.onerror = null;
      this.socket.close();
    }
    this.options.onClosed?.();
  }
}
