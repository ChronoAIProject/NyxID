import type {
  VoiceMediaAdapter,
  VoiceMediaEvent,
} from "./AssistantVoiceClient";
import type { VoiceProtocol } from "./voiceContracts";

type Listener = (event: { data?: unknown; track?: NativeAudioTrack }) => void;

interface NativeEvents {
  addEventListener(type: string, listener: Listener): void;
  removeEventListener(type: string, listener: Listener): void;
}

export interface NativeAudioTrack extends NativeEvents {
  kind: string;
  enabled: boolean;
  readyState: string;
  stop(): void;
  _setVolume(volume: number): void;
}

export interface NativeAudioStream {
  getTracks(): NativeAudioTrack[];
  getAudioTracks(): NativeAudioTrack[];
  release(releaseTracks?: boolean): void;
}

export interface NativeAudioChannel extends NativeEvents {
  close(): void;
}

export interface NativeAudioPeer extends NativeEvents {
  connectionState: string;
  iceConnectionState: string;
  iceGatheringState: string;
  localDescription: { type: string; sdp: string } | null;
  addTrack(track: NativeAudioTrack, stream: NativeAudioStream): unknown;
  createOffer(): Promise<{ type: "offer"; sdp: string }>;
  setLocalDescription(description: {
    type: "offer";
    sdp: string;
  }): Promise<void>;
  setRemoteDescription(description: {
    type: "answer";
    sdp: string;
  }): Promise<void>;
  createDataChannel(label: string): NativeAudioChannel;
  getStats(): Promise<Map<string, Record<string, unknown>>>;
  close(): void;
}

export interface NativeOpenAIDependencies {
  getUserMedia(): Promise<NativeAudioStream>;
  createPeer(): NativeAudioPeer;
  /** Injectable clocks keep cancellation/timeout tests deterministic. */
  setTimeout?: typeof setTimeout;
  clearTimeout?: typeof clearTimeout;
  setInterval?: typeof setInterval;
  clearInterval?: typeof clearInterval;
}

type Diagnostic = {
  stage: string;
  name: string;
  counts?: Record<string, number>;
};

const closedError = () => new Error("Voice audio was closed.");
const startupError = "Voice audio could not be started. Please try again.";
const connectionError = "WebRTC negotiation failed.";
let microphoneOwner: NativeOpenAIMedia | undefined;

/**
 * Ephemeral native WebRTC media. No account credentials, provider messages,
 * SDP, candidates, track labels, or recorded audio enter diagnostics.
 */
export class NativeOpenAIMedia implements VoiceMediaAdapter {
  private closed = true;
  private generation = 0;
  private peer?: NativeAudioPeer;
  private stream?: NativeAudioStream;
  private channel?: NativeAudioChannel;
  private remoteTracks = new Set<NativeAudioTrack>();
  private disposers: (() => void)[] = [];
  private waiters = new Set<(error: Error) => void>();
  private listeners = new Set<(event: VoiceMediaEvent) => void>();
  private timer?: ReturnType<typeof setInterval>;
  private statsPending = false;
  private inputEnabled = false;
  private held = false;
  private speakerMuted = false;
  private providerReady = false;
  private telemetry = { playback_ms: 0, playback_audible: false };
  private diagnostics: Diagnostic[] = [];
  private readonly clock;

  constructor(private readonly dependencies: NativeOpenAIDependencies) {
    this.clock = {
      setTimeout: dependencies.setTimeout ?? setTimeout,
      clearTimeout: dependencies.clearTimeout ?? clearTimeout,
      setInterval: dependencies.setInterval ?? setInterval,
      clearInterval: dependencies.clearInterval ?? clearInterval,
    };
  }

  getDiagnostics(): readonly Diagnostic[] {
    return this.diagnostics.map((row) => ({
      ...row,
      ...(row.counts ? { counts: { ...row.counts } } : {}),
    }));
  }

  private diagnostic(
    stage: string,
    name: string,
    counts?: Record<string, number>,
  ) {
    this.diagnostics = [
      ...this.diagnostics.filter((row) => row.stage !== stage).slice(-15),
      { stage, name, ...(counts ? { counts } : {}) },
    ];
  }

  private alive(generation: number) {
    return !this.closed && this.generation === generation;
  }

  private assertAlive(generation: number) {
    if (!this.alive(generation)) throw closedError();
  }

  private emit(event: VoiceMediaEvent) {
    for (const listener of this.listeners) {
      try {
        listener(event);
      } catch {
        // A subscriber must not prevent media teardown.
      }
    }
  }

  subscribe(listener: (event: VoiceMediaEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private listen(target: NativeEvents, type: string, listener: Listener) {
    target.addEventListener(type, listener);
    this.disposers.push(() => target.removeEventListener(type, listener));
  }

  private wait<T>(operation: Promise<T>, generation: number, ms: number) {
    return new Promise<T>((resolve, reject) => {
      let settled = false;
      const finish = (error?: Error, value?: T) => {
        if (settled) return;
        settled = true;
        this.clock.clearTimeout(timer);
        this.waiters.delete(cancel);
        if (error) reject(error);
        else if (!this.alive(generation)) reject(closedError());
        else resolve(value as T);
      };
      const cancel = (error: Error) => finish(error);
      const timer = this.clock.setTimeout(() => {
        const error = new Error(
          "Voice audio took too long to start. Please try again.",
        );
        finish(error);
        if (this.alive(generation)) this.fail(error.message);
      }, ms);
      this.waiters.add(cancel);
      operation.then(
        (value) => finish(undefined, value),
        () => finish(new Error(startupError)),
      );
      if (!this.alive(generation)) finish(closedError());
    });
  }

  async prepare(protocol: VoiceProtocol, signal: AbortSignal): Promise<string> {
    this.close();
    if (protocol !== "openai_live")
      throw new Error("Voice audio is not supported by this app runtime.");
    if (signal.aborted) throw closedError();
    if (microphoneOwner && microphoneOwner !== this)
      throw new Error("The microphone is already in use.");
    microphoneOwner = this;
    this.closed = false;
    const generation = ++this.generation;
    this.diagnostics = [];
    this.inputEnabled = false;
    this.held = this.speakerMuted = this.providerReady = false;
    const aborted = () => {
      if (this.alive(generation)) this.close();
    };
    signal.addEventListener("abort", aborted, { once: true });
    this.disposers.push(() => signal.removeEventListener("abort", aborted));
    try {
      this.diagnostic("microphone", "pending");
      // Native getUserMedia cannot be cancelled. A late result is always freed.
      const capture = this.dependencies.getUserMedia().then((stream) => {
        if (!this.alive(generation)) {
          this.releaseStream(stream);
          throw closedError();
        }
        this.stream = stream;
        for (const track of stream.getTracks()) track.enabled = false;
        return stream;
      });
      const stream = await this.wait(capture, generation, 20000);
      this.assertAlive(generation);
      const tracks = stream.getAudioTracks();
      const track = tracks[0];
      if (tracks.length !== 1 || !track || track.readyState !== "live")
        throw new Error(startupError);
      this.listen(track, "ended", () => {
        if (this.alive(generation))
          this.fail(
            "Microphone access ended. Start a new call when it is available.",
          );
      });
      this.diagnostic("microphone", "ready", { audio_tracks: tracks.length });
      const peer = this.dependencies.createPeer();
      this.peer = peer;
      const state = () => {
        if (!this.alive(generation)) return;
        const allowed = [
          "new",
          "connecting",
          "connected",
          "disconnected",
          "failed",
          "closed",
        ];
        if (allowed.includes(peer.connectionState))
          this.diagnostic("openai_peer", peer.connectionState);
        if (
          peer.connectionState === "failed" ||
          peer.connectionState === "closed" ||
          peer.iceConnectionState === "failed" ||
          peer.iceConnectionState === "closed"
        )
          this.fail(connectionError);
      };
      this.listen(peer, "connectionstatechange", state);
      this.listen(peer, "iceconnectionstatechange", state);
      this.listen(peer, "track", ({ track }) => {
        if (!this.alive(generation) || !track || track.kind !== "audio") return;
        if (track.readyState !== "live") return this.fail(connectionError);
        if (this.remoteTracks.has(track)) return;
        if (this.remoteTracks.size >= 4) return this.fail(connectionError);
        this.remoteTracks.add(track);
        this.listen(track, "ended", () => {
          if (this.alive(generation)) this.fail(connectionError);
        });
        this.applySpeaker();
      });
      peer.addTrack(track, stream);
      const channel = peer.createDataChannel("oai-events");
      this.channel = channel;
      this.listen(channel, "message", ({ data }) => {
        if (
          !this.alive(generation) ||
          this.providerReady ||
          typeof data !== "string" ||
          data.length > 262144
        )
          return;
        try {
          if (JSON.parse(data)?.type === "session.started") {
            this.providerReady = true;
            this.diagnostic("openai_datachannel", "ready");
            this.emit({ type: "provider-ready" });
          }
        } catch {
          /* Provider content is never retained or logged. */
        }
      });
      for (const event of ["error", "close"])
        this.listen(channel, event, () => {
          if (this.alive(generation)) this.fail(connectionError);
        });
      await this.wait(
        (async () => {
          const offer = await peer.createOffer();
          this.assertAlive(generation);
          await peer.setLocalDescription(offer);
        })(),
        generation,
        15000,
      );
      this.assertAlive(generation);
      await this.waitForICE(peer, generation);
      this.assertAlive(generation);
      if (track.readyState !== "live") throw new Error(startupError);
      const sdp = peer.localDescription?.sdp;
      if (typeof sdp !== "string" || !sdp.trim() || sdp.length > 131072)
        throw new Error(startupError);
      this.timer = this.clock.setInterval(
        () => void this.sample(generation),
        200,
      );
      return sdp;
    } catch (error) {
      if (this.alive(generation)) this.close();
      throw error;
    }
  }

  private async waitForICE(peer: NativeAudioPeer, generation: number) {
    if (peer.iceGatheringState === "complete") return;
    let changed: Listener;
    const complete = new Promise<void>((resolve) => {
      changed = () => {
        if (peer.iceGatheringState === "complete") resolve();
      };
      peer.addEventListener("icegatheringstatechange", changed);
      changed({});
    });
    try {
      await this.wait(complete, generation, 5000);
    } finally {
      peer.removeEventListener("icegatheringstatechange", changed!);
    }
  }

  async answer(sdp: string): Promise<void> {
    if (this.closed || !this.peer) throw closedError();
    if (!sdp.trim() || sdp.length > 131072)
      throw new Error("Invalid voice answer.");
    const generation = this.generation;
    try {
      await this.wait(
        this.peer.setRemoteDescription({ type: "answer", sdp }),
        generation,
        15000,
      );
    } catch (error) {
      if (this.alive(generation)) this.close();
      throw error;
    }
  }

  setInput(enabled: boolean): void {
    if (this.closed) return;
    this.inputEnabled = enabled;
    try {
      for (const track of this.stream?.getAudioTracks() ?? []) {
        if (track.readyState !== "live") return this.fail(startupError);
        track.enabled = enabled;
      }
    } catch {
      this.fail(startupError);
    }
  }

  private applySpeaker() {
    try {
      for (const track of this.remoteTracks) {
        track.enabled = !this.speakerMuted;
        track._setVolume(this.speakerMuted ? 0 : this.held ? 0.25 : 1);
      }
      if (this.speakerMuted) this.telemetry.playback_audible = false;
    } catch {
      this.fail(
        "Voice playback could not be started. Try another voice source or continue with text.",
      );
    }
  }

  hold(pressed: boolean): void {
    if (this.closed) return;
    this.held = pressed;
    this.applySpeaker();
  }

  muteSpeaker(muted: boolean): void {
    if (this.closed) return;
    this.speakerMuted = muted;
    this.applySpeaker();
  }

  async resume(): Promise<void> {
    if (this.closed) throw closedError();
    this.applySpeaker();
  }

  /** PCM is handled by the Grok adapter; never accept it on a native RTP call. */
  enqueue(_bytes: ArrayBuffer, _endMs: number): void {
    if (!this.closed) this.fail("Voice playback could not be verified.");
  }

  flush(): void {
    /* OpenAI interruption is controlled by the provider. */
  }

  playback() {
    return { ...this.telemetry };
  }

  private async sample(generation: number) {
    if (!this.alive(generation) || this.statsPending || !this.peer) return;
    const peer = this.peer;
    this.statsPending = true;
    try {
      const reports = await peer.getStats();
      if (!this.alive(generation)) return;
      let input = 0,
        output = 0,
        received = 0,
        sent = 0;
      let playbackMs = 0,
        count = 0;
      for (const report of reports.values()) {
        if (++count > 128) break;
        if (report.kind !== "audio" && report.mediaType !== "audio") continue;
        const level =
          typeof report.audioLevel === "number" &&
          Number.isFinite(report.audioLevel)
            ? Math.min(1, Math.max(0, report.audioLevel))
            : 0;
        if (report.type === "media-source" || report.type === "outbound-rtp") {
          input = Math.max(input, level);
          if (
            typeof report.bytesSent === "number" &&
            Number.isSafeInteger(report.bytesSent) &&
            report.bytesSent >= 0
          )
            sent = Math.max(sent, report.bytesSent);
        }
        if (report.type === "inbound-rtp") {
          output = Math.max(output, level);
          if (
            typeof report.bytesReceived === "number" &&
            Number.isSafeInteger(report.bytesReceived) &&
            report.bytesReceived >= 0
          )
            received = Math.max(received, report.bytesReceived);
          // Use an emitted-sample counter, never inferred wall-clock playback.
          const codec =
            typeof report.codecId === "string"
              ? reports.get(report.codecId)
              : undefined;
          if (
            typeof report.jitterBufferEmittedCount === "number" &&
            report.jitterBufferEmittedCount >= 0 &&
            Number.isFinite(report.jitterBufferEmittedCount) &&
            typeof codec?.clockRate === "number" &&
            codec.clockRate > 0
          )
            playbackMs = Math.max(
              playbackMs,
              Math.floor(
                (report.jitterBufferEmittedCount / codec.clockRate) * 1000,
              ),
            );
        }
      }
      const audible =
        !this.speakerMuted &&
        peer.connectionState === "connected" &&
        [...this.remoteTracks].some(
          (track) => track.readyState === "live" && track.enabled,
        ) &&
        received > 0;
      this.telemetry = {
        playback_ms: Math.max(
          this.telemetry.playback_ms,
          Math.min(1_830_000, playbackMs),
        ),
        playback_audible: audible,
      };
      this.diagnostic("openai_stats", "ready", {
        audio_reports: Math.min(count, 128),
      });
      this.diagnostic("openai_rtp_outbound", "ready", { bytes: sent });
      this.diagnostic("openai_rtp_inbound", "ready", { bytes: received });
      this.emit({
        type: "level",
        input: this.inputEnabled ? input : 0,
        output: this.speakerMuted ? 0 : output,
      });
    } catch {
      // Meters are optional and must not terminate an otherwise healthy call.
      if (this.alive(generation)) this.telemetry.playback_audible = false;
    } finally {
      if (this.generation === generation) this.statsPending = false;
    }
  }

  private fail(message: string) {
    if (this.closed) return;
    this.close();
    this.emit({ type: "error", message });
  }

  private releaseStream(stream: NativeAudioStream) {
    for (const track of stream.getTracks()) {
      try {
        track.stop();
      } catch {
        /* Continue freeing other native resources. */
      }
    }
    try {
      stream.release(true);
    } catch {
      /* Native process may already be gone. */
    }
  }

  close(): void {
    this.closed = true;
    ++this.generation;
    if (microphoneOwner === this) microphoneOwner = undefined;
    if (this.timer !== undefined) this.clock.clearInterval(this.timer);
    this.timer = undefined;
    this.statsPending = false;
    for (const dispose of this.disposers.splice(0)) {
      try {
        dispose();
      } catch {
        /* Continue teardown after native failure. */
      }
    }
    for (const cancel of [...this.waiters]) cancel(closedError());
    this.waiters.clear();
    try {
      this.channel?.close();
    } catch {
      /* Continue teardown. */
    }
    this.channel = undefined;
    for (const track of this.remoteTracks) {
      try {
        track.enabled = false;
        track.stop();
      } catch {
        /* Continue teardown. */
      }
    }
    this.remoteTracks.clear();
    try {
      this.peer?.close();
    } catch {
      /* Continue teardown. */
    }
    this.peer = undefined;
    if (this.stream) this.releaseStream(this.stream);
    this.stream = undefined;
    this.inputEnabled =
      this.held =
      this.speakerMuted =
      this.providerReady =
        false;
    this.telemetry = { playback_ms: 0, playback_audible: false };
    this.emit({ type: "level", input: 0, output: 0 });
  }
}
