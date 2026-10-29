import { useEffect, useMemo, useRef } from "react";
import { StyleSheet } from "react-native";
import { WebView } from "react-native-webview";
import type {
  VoiceMediaAdapter,
  VoiceMediaEvent,
} from "./AssistantVoiceClient";
import {
  createVoiceMediaDocument,
  voicePcmFromBase64,
  voicePcmToBase64,
} from "./voiceMediaDocument";

let microphoneOwner: NativeVoiceMedia | undefined;

type Pending = {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
};

type MediaDiagnostic = {
  stage: string;
  name: string;
  counts?: Record<string, number>;
};

const closedError = () => new Error("Voice audio was closed.");

/** No authentication material is ever passed to the local media document. */
export class NativeVoiceMedia implements VoiceMediaAdapter {
  private send?: (message: Record<string, unknown>) => void;
  private ready = false;
  private closed = true;
  private generation = 0;
  private sequence = 0;
  private failedGeneration = -1;
  private pending = new Map<number, Pending>();
  private readyWaiters = new Set<{
    resolve: () => void;
    reject: (error: Error) => void;
  }>();
  private listeners = new Set<(event: VoiceMediaEvent) => void>();
  private telemetry = { playback_ms: 0, playback_audible: false };
  private diagnostics: MediaDiagnostic[] = [];

  /** Bounded capability diagnostics only; never SDP, candidates, IPs, or audio. */
  getDiagnostics(): readonly MediaDiagnostic[] {
    return this.diagnostics.map((row) => ({
      ...row,
      ...(row.counts ? { counts: { ...row.counts } } : {}),
    }));
  }

  connect(send: (message: Record<string, unknown>) => void): () => void {
    this.send = send;
    return () => {
      if (this.send !== send) return;
      this.close();
      this.ready = false;
      this.send = undefined;
    };
  }

  receive(raw: string): void {
    if (typeof raw !== "string" || raw.length > 200000) {
      this.unavailable();
      return;
    }
    let event: Record<string, unknown>;
    try {
      const parsed: unknown = JSON.parse(raw);
      if (!parsed || typeof parsed !== "object") throw new Error();
      event = parsed as Record<string, unknown>;
    } catch {
      this.unavailable();
      return;
    }
    if (event.type === "ready") {
      if (this.ready && !this.closed) {
        this.unavailable();
        return;
      }
      this.ready = true;
      for (const waiter of this.readyWaiters) waiter.resolve();
      this.readyWaiters.clear();
      return;
    }
    if (this.closed || event.generation !== this.generation) return;
    if (event.type === "diagnostic") {
      const stages = new Set([
        "runtime",
        "microphone",
        "audio_context",
        "audio_worklet",
        "loopback_offer",
        "loopback_offer_ice",
        "loopback_receiver_offer",
        "loopback_answer",
        "loopback_answer_ice",
        "loopback_sender_answer",
        "loopback_connected",
        "loopback_sender_state",
        "loopback_receiver_state",
        "loopback_sender_ice",
        "loopback_receiver_ice",
        "loopback_receiver_track",
      ]);
      const names = new Set([
        "pending",
        "ready",
        "Error",
        "TypeError",
        "OperationError",
        "InvalidStateError",
        "NotSupportedError",
        "NotAllowedError",
        "AbortError",
        "NotFoundError",
        "UnknownError",
        "SecurityError",
        "TimeoutError",
        "new",
        "connecting",
        "connected",
        "disconnected",
        "checking",
        "completed",
        "failed",
        "closed",
      ]);
      if (
        typeof event.stage === "string" &&
        stages.has(event.stage) &&
        typeof event.name === "string" &&
        names.has(event.name)
      ) {
        const counts: Record<string, number> = {};
        const allowedCounts = [
          "candidates",
          "host",
          "srflx",
          "relay",
          "mdns",
          "udp",
          "tcp",
          "ipv4",
          "ipv6",
          "secure_context",
          "opaque_document_origin",
          "opaque_location_origin",
          "document_visible",
        ];
        if (event.counts && typeof event.counts === "object") {
          for (const [key, value] of Object.entries(event.counts)) {
            if (
              allowedCounts.includes(key) &&
              Number.isSafeInteger(value) &&
              (value as number) >= 0 &&
              (value as number) <= 256
            )
              counts[key] = value as number;
          }
        }
        this.diagnostics = [
          ...this.diagnostics.slice(-31),
          {
            stage: event.stage,
            name: event.name,
            ...(Object.keys(counts).length ? { counts } : {}),
          },
        ];
      }
    } else if (event.type === "response") {
      const request = this.pending.get(event.requestId as number);
      if (!request) return;
      this.pending.delete(event.requestId as number);
      clearTimeout(request.timer);
      if (typeof event.error === "string")
        request.reject(new Error(event.error));
      else request.resolve(event.result);
    } else if (event.type === "provider-ready") {
      this.emit({ type: "provider-ready" });
    } else if (event.type === "pcm") {
      try {
        if (
          !Number.isSafeInteger(event.sequence) ||
          typeof event.bytes !== "string"
        )
          throw new Error();
        const bytes = voicePcmFromBase64(event.bytes);
        if (bytes.byteLength !== 960) throw new Error();
        this.command({ type: "pcm-ack", sequence: event.sequence });
        this.emit({ type: "pcm", bytes });
      } catch {
        this.unavailable();
      }
    } else if (event.type === "playback") {
      if (
        Number.isSafeInteger(event.playback_ms) &&
        (event.playback_ms as number) >= 0 &&
        (event.playback_ms as number) <= 1830000 &&
        typeof event.playback_audible === "boolean"
      )
        this.telemetry = {
          playback_ms: event.playback_ms as number,
          playback_audible: event.playback_audible,
        };
    } else if (event.type === "error") {
      // Engine messages are fixed strings; exception and provider text stay private.
      const allowed = new Set([
        "Microphone access was denied. Enable it in Settings to use voice.",
        "No microphone is available.",
        "Voice audio is not supported by this app runtime.",
        "Voice audio could not be started. Please try again.",
        "Voice audio connection is too slow. Please reconnect.",
        "Voice playback could not be started. Try another voice source or continue with text.",
        "Voice playback could not be verified.",
        "WebRTC negotiation failed.",
        "Voice ended when the app left the foreground.",
      ]);
      this.unavailable(
        typeof event.message === "string" && allowed.has(event.message)
          ? event.message
          : undefined,
      );
    }
  }

  unavailable(
    message = "Voice audio connection was lost. Please try again.",
  ): void {
    if (this.closed) return;
    if (this.failedGeneration !== this.generation) {
      this.failedGeneration = this.generation;
      this.emit({ type: "error", message });
    }
    this.close();
  }

  private emit(event: VoiceMediaEvent) {
    for (const listener of this.listeners) {
      try {
        listener(event);
      } catch {
        // A UI subscriber must not prevent media teardown or PCM backpressure.
      }
    }
  }

  subscribe(listener: (event: VoiceMediaEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private command(message: Record<string, unknown>): void {
    if (this.closed || !this.ready) return;
    try {
      this.send?.({ ...message, generation: this.generation });
    } catch {
      this.unavailable();
    }
  }

  private request(message: Record<string, unknown>): Promise<unknown> {
    if (this.closed || !this.ready || !this.send)
      return Promise.reject(closedError());
    const requestId = ++this.sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(requestId);
        reject(
          new Error("Voice audio took too long to start. Please try again."),
        );
        this.unavailable(
          "Voice audio took too long to start. Please try again.",
        );
      }, 20000);
      this.pending.set(requestId, { resolve, reject, timer });
      this.command({ ...message, requestId });
    });
  }

  async prepare(
    protocol: "openai_live" | "xai_realtime" | "dictation",
    signal: AbortSignal,
  ): Promise<string> {
    this.close();
    if (microphoneOwner && microphoneOwner !== this)
      throw new Error("The microphone is already in use.");
    microphoneOwner = this;
    this.closed = false;
    this.diagnostics = [];
    const generation = ++this.generation;
    const aborted = () => {
      if (generation === this.generation) this.close();
    };
    signal.addEventListener("abort", aborted, { once: true });
    try {
      if (signal.aborted) throw closedError();
      if (!this.ready) {
        await new Promise<void>((resolve, reject) => {
          const waiter = {
            resolve: () => {
              clearTimeout(timer);
              this.readyWaiters.delete(waiter);
              resolve();
            },
            reject: (error: Error) => {
              clearTimeout(timer);
              this.readyWaiters.delete(waiter);
              reject(error);
            },
          };
          const timer = setTimeout(
            () =>
              waiter.reject(
                new Error("Voice audio is not ready. Please try again."),
              ),
            15000,
          );
          this.readyWaiters.add(waiter);
        });
      }
      if (signal.aborted || this.closed || generation !== this.generation)
        throw closedError();
      const result = await this.request({ type: "prepare", protocol });
      if (signal.aborted || this.closed || generation !== this.generation)
        throw closedError();
      if (
        typeof result !== "string" ||
        result.length > 131072 ||
        (protocol === "openai_live" && !result)
      )
        throw new Error("Voice audio returned an invalid offer.");
      return result;
    } catch (error) {
      if (generation === this.generation) this.close();
      throw error;
    } finally {
      signal.removeEventListener("abort", aborted);
    }
  }

  async answer(sdp: string): Promise<void> {
    if (this.closed) return;
    if (!sdp || sdp.length > 131072) throw new Error("Invalid voice answer.");
    await this.request({ type: "answer", sdp });
  }
  setInput(enabled: boolean): void {
    this.command({ type: "input", enabled });
  }
  hold(pressed: boolean): void {
    this.command({ type: "hold", pressed });
  }
  muteSpeaker(muted: boolean): void {
    this.command({ type: "speaker", muted });
  }
  async resume(): Promise<void> {
    if (!this.closed) await this.request({ type: "resume" });
  }
  enqueue(bytes: ArrayBuffer, endMs: number): void {
    if (this.closed) return;
    if (!Number.isSafeInteger(endMs) || endMs < 0 || endMs > 1830000) {
      this.unavailable();
      return;
    }
    try {
      this.command({
        type: "enqueue",
        bytes: voicePcmToBase64(bytes),
        end_ms: endMs,
      });
    } catch {
      this.unavailable();
    }
  }
  flush(): void {
    this.command({ type: "flush" });
  }
  playback(): { playback_ms: number; playback_audible: boolean } {
    return { ...this.telemetry };
  }
  close(): void {
    const active = !this.closed && this.ready;
    this.closed = true;
    if (microphoneOwner === this) microphoneOwner = undefined;
    if (active) {
      try {
        this.send?.({ type: "close", generation: this.generation });
      } catch {
        // A terminated WebContent process already owns no usable media bridge.
      }
    }
    this.telemetry = { playback_ms: 0, playback_audible: false };
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(closedError());
    }
    this.pending.clear();
    for (const waiter of this.readyWaiters) waiter.reject(closedError());
    this.readyWaiters.clear();
  }
}

export function VoiceMediaBridge({
  media,
  origin,
}: {
  media: NativeVoiceMedia;
  origin: string;
}) {
  const webView = useRef<WebView>(null);
  const loaded = useRef(false);
  const source = useMemo(
    () => ({ html: createVoiceMediaDocument(), baseUrl: origin }),
    [origin],
  );
  useEffect(() => {
    loaded.current = false;
    return media.connect((message) => {
      const serialized = JSON.stringify(message).replace(/</g, "\\u003c");
      webView.current?.injectJavaScript(
        `window.__nyxVoiceMessage && window.__nyxVoiceMessage(${serialized}); true;`,
      );
    });
  }, [media, origin]);

  return (
    <WebView
      ref={webView}
      source={source}
      style={styles.engine}
      containerStyle={styles.engine}
      pointerEvents="none"
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      originWhitelist={["*"]}
      javaScriptEnabled
      allowsInlineMediaPlayback
      mediaPlaybackRequiresUserAction={false}
      mediaCapturePermissionGrantType="grantIfSameHostElseDeny"
      sharedCookiesEnabled={false}
      thirdPartyCookiesEnabled={false}
      cacheEnabled={false}
      setSupportMultipleWindows={false}
      onShouldStartLoadWithRequest={({ url }) =>
        !loaded.current &&
        (url === "about:blank" || url === origin || url === `${origin}/`)
      }
      onLoadEnd={() => {
        loaded.current = true;
      }}
      onMessage={({ nativeEvent }) => media.receive(nativeEvent.data)}
      onError={() => media.unavailable()}
      onContentProcessDidTerminate={() => media.unavailable()}
      onRenderProcessGone={() => media.unavailable()}
    />
  );
}

const styles = StyleSheet.create({
  engine: {
    position: "absolute",
    width: 1,
    height: 1,
    left: 0,
    top: 0,
    opacity: 0,
  },
});
