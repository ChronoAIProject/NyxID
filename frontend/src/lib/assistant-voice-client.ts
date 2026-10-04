import { GrokAudio } from "./grok-audio";
import { api, apiClient, apiUrl, ApiError } from "./api-client";
import {
  voiceStartedSchema,
  voiceSessionSchema,
  voiceSnapshotSchema,
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
  constructor(
    thread: string,
    onSnapshot: (value: VoiceSnapshot) => void,
    onError: (message: string) => void,
    onClosed: () => void = () => {},
  ) {
    this.thread = thread;
    this.onSnapshot = onSnapshot;
    this.onError = onError;
    this.onClosed = onClosed;
  }
  private path() {
    return `/assistant/nyxagent/conversations/${this.thread}/voice-sessions`;
  }
  async start(
    preferences: VoicePreferences,
    protocol: "openai_live" | "xai_realtime" = "openai_live",
  ) {
    this.preferences = preferences;
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
        const result = voiceStartedSchema.parse(
          await apiClient(this.path(), {
            method: "POST",
            signal: this.abort.signal,
            body: {
              client_request_id: crypto.randomUUID(),
              preferences,
              sdp_offer: "",
            },
          }),
        );
        this.session = result.session;
        if (this.closed) {
          await this.end();
          return;
        }
        this.started = true;
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
          this.onError("Voice connection lost. Start a new call to reconnect.");
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
      if (this.closed) return;
      const result = voiceStartedSchema.parse(
        await apiClient(this.path(), {
          method: "POST",
          signal: this.abort.signal,
          body: {
            client_request_id: crypto.randomUUID(),
            preferences,
            sdp_offer: peer.localDescription?.sdp,
          },
        }),
      );
      this.session = result.session;
      if (this.closed) {
        await this.end();
        return;
      }
      await peer.setRemoteDescription({
        type: "answer",
        sdp: result.sdp_answer,
      });
      this.openControls();
    } catch (error) {
      if (!this.closed)
        this.onError(
          error instanceof ApiError &&
            error.status === 409 &&
            error.message === "End the current call first"
            ? "End the current call first"
            : "Voice could not start. Check microphone access, your connection and voice credits.",
        );
      await this.end();
    }
  }
  private openControls() {
    if (!this.session || this.closed) return;
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
        const snapshot = voiceSnapshotSchema.parse(event);
        if (
          this.closed ||
          snapshot.session.id !== this.session?.id ||
          snapshot.session.generation !== this.session.generation
        )
          return;
        this.controlsReady = true;
        this.session = snapshot.session;
        this.onSnapshot(snapshot);
        this.updateInput();
        if (
          snapshot.session.state === "closed" ||
          snapshot.session.state === "failed"
        )
          this.dispose();
      } catch {
        this.onError("Voice state could not be verified.");
        void this.end();
      }
    };
    socket.onclose = () => {
      if (!this.closed) {
        this.onError("Voice control connection lost.");
        void this.end();
      }
    };
    this.heartbeat = setInterval(() => {
      if (socket.readyState === WebSocket.OPEN && this.session)
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
    if (this.heartbeat) clearInterval(this.heartbeat);
    this.media?.getTracks().forEach((t) => t.stop());
    this.peer?.close();
    this.grok?.close();
    this.controlSocket?.close();
    this.audio?.pause();
    if (this.audio) this.audio.srcObject = null;
  }
}
