import { voiceStartError, type VoiceStartStep } from "./voice-start-error";
import { GrokAudio } from "./grok-audio";
import { api, apiClient, apiUrl, ApiError } from "./api-client";
import {
  voiceStartedSchema,
  voiceStartFailureSchema,
  voiceSessionSchema,
  voiceSnapshotSchema,
  activeVoiceSessionSchema,
  type VoicePreferences,
  type VoiceSession,
  type VoiceSnapshot,
} from "@/schemas/assistant-voice";

/** Media is ephemeral; only the server's sideband can admit work or usage. */
export class AssistantVoiceClient {
  private grok?: GrokAudio;
  private audioEnd?: number;
  private peer?: RTCPeerConnection;
  private media?: MediaStream;
  private controlSocket?: WebSocket;
  private audio?: HTMLAudioElement;
  private session?: VoiceSession;
  private abort = new AbortController();
  private heartbeat?: ReturnType<typeof setInterval>;
  private reconnectTimer?: ReturnType<typeof setTimeout>;
  private reconnectAttempt = 0;
  private started = false;
  private controlsReady = false;
  private closed = false;
  private held = false;
  private speakerMuted = false;
  get isClosed() {
    return this.closed;
  }
  private revision = 0;
  private preferences?: VoicePreferences;
  private thread: string;
  private onSnapshot: (value: VoiceSnapshot) => void;
  private onError: (message: string) => void;
  private onClosed: () => void;
  private onConflict: (session: VoiceSession | null) => void;
  constructor(
    thread: string,
    onSnapshot: (value: VoiceSnapshot) => void,
    onError: (message: string) => void,
    onClosed: () => void = () => {},
    onConflict: (session: VoiceSession | null) => void = () => {},
  ) {
    this.thread = thread;
    this.onSnapshot = onSnapshot;
    this.onError = onError;
    this.onClosed = onClosed;
    this.onConflict = onConflict;
  }
  private path() {
    return `/assistant/nyxagent/conversations/${this.thread}/voice-sessions`;
  }
  async start(
    preferences: VoicePreferences,
    protocol: "openai_live" | "xai_realtime" = "openai_live",
  ) {
    this.preferences = preferences;
    let step: VoiceStartStep = "microphone";
    try {
      this.audio = document.createElement("audio");
      this.audio.autoplay = true;
      this.audio.setAttribute("playsinline", "");
      const media = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
        video: false,
      });
      if (this.closed) {
        media.getTracks().forEach((t) => t.stop());
        return;
      }
      this.media = media;
      media.getAudioTracks().forEach((t) => {
        t.enabled = false;
      });
      step = "webrtc";
      if (protocol === "xai_realtime") {
        if (preferences.input_mode !== "push_to_talk")
          throw new Error("Grok requires Hold to talk");
        this.grok = new GrokAudio(
          this.audio,
          (bytes) => {
            const socket = this.controlSocket;
            if (
              !this.closed &&
              this.held &&
              socket?.readyState === WebSocket.OPEN
            ) {
              if (socket.bufferedAmount > 96000) {
                this.onError(
                  "Voice audio connection is too slow. Please reconnect.",
                );
                void this.end();
              } else socket.send(bytes);
            }
          },
          () => {
            if (!this.closed) {
              this.onError(
                "Voice playback could not be verified. Use headphones and restart the call.",
              );
              void this.end();
            }
          },
        );
        await this.grok.start(media);
        if (this.closed) return;
        step = "server";
        const raw = await apiClient(this.path(), {
          method: "POST",
          signal: this.abort.signal,
          body: {
            client_request_id: crypto.randomUUID(),
            preferences,
            sdp_offer: "",
          },
        });
        step = "response";
        const result = voiceStartedSchema.parse(raw);
        this.session = result.session;
        if (this.closed) {
          await this.end();
          return;
        }
        this.started = true;
        step = "transport";
        this.openControls();
        return;
      }
      const peer = new RTCPeerConnection();
      this.peer = peer;
      media.getTracks().forEach((t) => peer.addTrack(t, media));
      peer.ontrack = ({ streams, track }) => {
        if (!this.audio || this.closed) return;
        this.audio.srcObject = streams[0] ?? new MediaStream([track]);
        void this.resumeAudio();
      };
      peer.onconnectionstatechange = () => {
        if (peer.connectionState === "failed") {
          this.onError(
            this.started && this.controlsReady
              ? "Voice connection lost. Start a new call to reconnect."
              : "WebRTC negotiation failed",
          );
          void this.end();
        }
      };
      const events = peer.createDataChannel("oai-events");
      events.onmessage = ({ data }) => {
        if (this.closed || typeof data !== "string" || data.length > 262144)
          return;
        try {
          const event: unknown = JSON.parse(data);
          if (
            typeof event === "object" &&
            event !== null &&
            "type" in event &&
            event.type === "session.started"
          ) {
            this.started = true;
            this.updateInput();
          }
        } catch {
          /* No provider payload is logged or sent to the backend. */
        }
      };
      await peer.setLocalDescription(await peer.createOffer());
      await this.waitForIce(peer);
      if (this.closed) return;
      step = "server";
      const raw = await apiClient(this.path(), {
        method: "POST",
        signal: this.abort.signal,
        body: {
          client_request_id: crypto.randomUUID(),
          preferences,
          sdp_offer: peer.localDescription?.sdp,
        },
      });
      step = "response";
      const result = voiceStartedSchema.parse(raw);
      this.session = result.session;
      if (this.closed) {
        await this.end();
        return;
      }
      step = "webrtc";
      await peer.setRemoteDescription({
        type: "answer",
        sdp: result.sdp_answer,
      });
      step = "transport";
      this.openControls();
    } catch (error) {
      if (!this.closed) {
        if (error instanceof ApiError && error.status === 409) {
          try {
            const active = activeVoiceSessionSchema.parse(
              await api.get(`${this.path()}/active`),
            );
            this.onConflict(active.session);
          } catch {
            this.onConflict(null);
          }
        }
        this.onError(voiceStartError(error, step));
      }
      await this.end();
    }
  }

  /** Attach only the browser control stream to a durable server call. */
  rejoin(session: VoiceSession) {
    if (this.closed || !session.resumable) return;
    this.session = session;
    this.started = true;
    this.controlsReady = false;
    this.reconnectAttempt = 0;
    this.openControls();
  }
  private waitForIce(peer: RTCPeerConnection): Promise<void> {
    if (peer.iceGatheringState === "complete") return Promise.resolve();
    return new Promise((resolve, reject) => {
      const finish = (error?: Error) => {
        clearTimeout(timer);
        peer.removeEventListener("icegatheringstatechange", changed);
        this.abort.signal.removeEventListener("abort", aborted);
        if (error) reject(error);
        else resolve();
      };
      const changed = () => {
        if (peer.iceGatheringState === "complete") finish();
      };
      const aborted = () => finish(new Error("Voice ended"));
      const timer = setTimeout(
        () => finish(new Error("ICE gathering timed out")),
        5000,
      );
      peer.addEventListener("icegatheringstatechange", changed);
      this.abort.signal.addEventListener("abort", aborted, { once: true });
      if (this.abort.signal.aborted) aborted();
      else changed();
    });
  }
  private openControls() {
    if (!this.session || this.closed) return;
    if (this.heartbeat) clearInterval(this.heartbeat);
    this.reconnectTimer = undefined;
    this.controlsReady = false;
    const url = new URL(
      apiUrl(`${this.path()}/${this.session.id}/stream`),
      window.location.href,
    );
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    const socket = new WebSocket(url);
    this.controlSocket = socket;
    socket.binaryType = "arraybuffer";
    socket.onmessage = ({ data }) => {
      if (this.closed) return;
      try {
        if (this.grok && data instanceof ArrayBuffer) {
          if (
            this.audioEnd === undefined ||
            data.byteLength > 1920 ||
            data.byteLength % 2 !== 0
          )
            throw new Error("Invalid PCM frame");
          this.grok.enqueue(data, this.audioEnd);
          this.audioEnd = undefined;
          return;
        }
        const event = JSON.parse(String(data)) as {
          type?: string;
          generation?: number;
          end_ms?: number;
        };
        if (
          this.grok &&
          (event.type === "audio" || event.type === "audio_flush")
        ) {
          if (event.generation !== this.session?.generation || this.closed)
            return;
          if (event.type === "audio_flush") {
            this.grok.flush();
            this.audioEnd = undefined;
            return;
          }
          if (
            this.audioEnd !== undefined ||
            !Number.isSafeInteger(event.end_ms) ||
            event.end_ms! < 0 ||
            event.end_ms! > 1830000
          )
            throw new Error("Invalid playback watermark");
          this.audioEnd = event.end_ms;
          return;
        }
        if (event.type === "start_failed") {
          const failure = voiceStartFailureSchema.parse(event);
          this.onError(
            voiceStartError(new ApiError(503, failure.error), "server"),
          );
          void this.end();
          return;
        }
        const snapshot = voiceSnapshotSchema.parse(event);
        if (this.closed || snapshot.session.id !== this.session?.id)
          return;
        this.controlsReady = true;
        this.reconnectAttempt = 0;
        this.session = snapshot.session;
        this.onSnapshot(snapshot);
        this.updateInput();
        if (snapshot.session.state === "closed" || snapshot.session.state === "failed") {
          if (snapshot.session.end_reason === "server_update")
            this.onError("The call ended during a server update; start a new call");
          this.dispose();
        }
      } catch {
        this.onError("Voice state could not be verified.");
        void this.end();
      }
    };
    socket.onclose = () => {
      if (this.closed || socket !== this.controlSocket) return;
      if (this.heartbeat) clearInterval(this.heartbeat);
      this.controlsReady = false;
      if (this.reconnectAttempt >= 6) {
        this.onError("Voice control connection could not reconnect.");
        return;
      }
      const delays = [1000, 2000, 4000, 8000, 15000, 30000];
      const delay = delays[this.reconnectAttempt++];
      this.reconnectTimer = setTimeout(() => {
        if (!this.closed) this.openControls();
      }, delay);
    };
    this.heartbeat = setInterval(() => {
      if (socket.readyState === WebSocket.OPEN && this.session && this.controlsReady)
        socket.send(
          JSON.stringify({
            type: "heartbeat",
            generation: this.session.generation,
            revision: ++this.revision,
            playback_ms:
              this.grok?.watermark() ??
              Math.max(0, Math.floor((this.audio?.currentTime ?? 0) * 1000)),
            playback_audible:
              !!this.audio &&
              !this.audio.paused &&
              !this.audio.muted &&
              this.audio.volume > 0,
          }),
        );
    }, 1000);
  }
  private updateInput() {
    const enabled =
      !this.closed &&
      this.started &&
      this.controlsReady &&
      this.session?.state === "active" &&
      !this.session.muted &&
      !this.session.input_muted &&
      (this.preferences?.input_mode === "automatic" || this.held);
    this.media?.getAudioTracks().forEach((t) => {
      t.enabled = enabled;
    });
    this.grok?.capture(!!enabled);
  }
  hold(pressed: boolean) {
    if (this.closed || this.held === pressed) return;
    if (this.grok) {
      if (
        pressed &&
        (this.session?.state !== "active" ||
          this.session.muted ||
          this.session.input_muted ||
          !this.controlsReady)
      )
        return;
      if (this.controlSocket?.readyState === WebSocket.OPEN)
        this.controlSocket.send(
          JSON.stringify({ type: pressed ? "ptt_begin" : "ptt_commit" }),
        );
      if (this.audio) this.audio.volume = pressed ? 0.25 : 1;
    }
    this.held = pressed;
    this.updateInput();
  }
  muteSpeaker(muted: boolean) {
    this.speakerMuted = muted;
    if (this.audio) this.audio.muted = muted;
  }
  async resumeAudio() {
    if (this.audio) this.audio.muted = this.speakerMuted;
    try {
      await this.grok?.resume();
      await this.audio?.play();
    } catch {
      this.onError("Tap Resume audio to hear the assistant.");
    }
  }
  async mute(muted: boolean) {
    if (!this.session || this.closed) return;
    if (muted) this.hold(false);
    if (muted)
      this.media?.getAudioTracks().forEach((t) => {
        t.enabled = false;
      });
    try {
      this.session = voiceSessionSchema.parse(
        await api.post(`${this.path()}/${this.session.id}/control`, {
          command_id: crypto.randomUUID(),
          expected_revision: this.session.control_revision,
          action: muted ? "mute" : "unmute",
        }),
      );
      this.updateInput();
    } catch {
      this.onError("Voice controls changed. End this call and reconnect.");
    }
  }
  async stopTask(id: string) {
    try {
      await api.post(
        `/assistant/nyxagent/conversations/${this.thread}/voice-requests/${id}/cancel`,
        {},
      );
    } catch {
      this.onError("The task could not be stopped. Try again from the thread.");
    }
  }
  async end() {
    this.closed = true;
    this.abort.abort();
    this.onClosed();
    this.media?.getTracks().forEach((t) => {
      t.enabled = false;
      t.stop();
    });
    this.audio?.pause();
    this.grok?.close();
    const session = this.session;
    this.session = undefined;
    if (!session) {
      this.dispose();
      return;
    }
    try {
      await api.post(`${this.path()}/${session.id}/control`, {
        command_id: crypto.randomUUID(),
        expected_revision: session.control_revision,
        action: "end",
      });
    } catch {
      /* Server heartbeat expiry still closes the call. Never retry session creation. */
    } finally {
      setTimeout(() => this.dispose(), 5000);
    }
  }
  private dispose() {
    this.onClosed();
    this.closed = true;
    this.abort.abort();
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    if (this.heartbeat) clearInterval(this.heartbeat);
    this.media?.getTracks().forEach((t) => t.stop());
    this.peer?.close();
    this.grok?.close();
    this.controlSocket?.close();
    this.audio?.pause();
    if (this.audio) this.audio.srcObject = null;
  }
}
