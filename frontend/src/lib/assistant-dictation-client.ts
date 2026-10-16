import { apiUrl } from "./api-client";
import {
  AudioActivityMeter,
  MicrophoneCapture,
  microphoneError,
  type AudioActivity,
} from "./microphone-capture";
import { DictationPcm } from "./dictation-pcm";
import type { VoicePreferences } from "@/schemas/assistant-voice";
import { z } from "zod";

const eventSchema = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("ready"),
    session_id: z.string().uuid(),
    model: z.literal("gpt-live-transcribe"),
  }),
  z.object({
    type: z.enum(["delta", "completed"]),
    item_id: z.string().min(1).max(256),
    text: z.string().max(32_000),
    order: z.number().int().nonnegative(),
  }),
  z.object({ type: z.literal("stopped"), confirmed: z.boolean() }),
  z.object({
    type: z.literal("error"),
    error: z.object({ message: z.string() }),
  }),
]);
export type DictationText = {
  final: string;
  interim: string;
  parts?: { text: string; complete: boolean; order: number }[];
};

/** Captions are local review text. This client has no chat send/history API. */
export class AssistantDictationClient {
  private capture?: MicrophoneCapture;
  private meter?: AudioActivityMeter;
  private pcm?: DictationPcm;
  private socket?: WebSocket;
  private finishing = false;
  private closed = false;
  private ready = false;
  private timeout?: ReturnType<typeof setTimeout>;
  private items = new Map<
    string,
    { text: string; complete: boolean; order: number }
  >();
  private callbacks: {
    text: (text: DictationText) => void;
    activity: (value: AudioActivity) => void;
    state: (
      state:
        | "requesting"
        | "connecting"
        | "capturing"
        | "finalizing"
        | "review",
    ) => void;
    error: (message: string) => void;
  };
  constructor(callbacks: AssistantDictationClient["callbacks"]) {
    this.callbacks = callbacks;
  }
  async start(
    target:
      | string
      | (() => Promise<{ thread: string; preferences: VoicePreferences }>),
    preferences?: VoicePreferences,
    deviceId?: string,
  ) {
    let microphoneReady = false;
    try {
      this.callbacks.state("requesting");
      this.capture = new MicrophoneCapture(() => {
        this.callbacks.error(
          "Microphone permission ended. Review the text already captured.",
        );
        this.stop();
      }, deviceId);
      // Observe cancellation rejection even if later gesture-time setup throws.
      void this.capture.ready.catch(() => {});
      this.pcm = new DictationPcm();
      this.meter = new AudioActivityMeter(this.callbacks.activity);
      const stream = await this.capture.ready;
      if (this.finishing || this.closed) return;
      this.meter.watch(stream, "input");
      microphoneReady = true;
      this.callbacks.state("connecting");
      const prepared =
        typeof target === "string"
          ? { thread: target, preferences: preferences! }
          : await target();
      if (this.finishing || this.closed) return;
      const { thread } = prepared;
      const url = new URL(
        apiUrl(`/assistant/nyxagent/conversations/${thread}/dictation`),
        location.href,
      );
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      const socket = new WebSocket(url);
      this.socket = socket;
      this.timeout = setTimeout(
        () => this.fail("Dictation could not connect. Try again."),
        15_000,
      );
      socket.onopen = () => {
        if (this.closed) {
          socket.close();
          return;
        }
        socket.send(
          JSON.stringify({
            client_request_id: crypto.randomUUID(),
            preferences: prepared.preferences,
          }),
        );
      };
      socket.onmessage = ({ data }) => {
        if (this.closed || typeof data !== "string" || data.length > 262144)
          return;
        try {
          const event = eventSchema.parse(JSON.parse(data));
          if (event.type === "ready") {
            this.ready = true;
            clearTimeout(this.timeout);
            if (this.finishing) {
              this.requestStop();
              return;
            }
            void this.pcm!.start(stream, (bytes) => {
              if (
                this.finishing ||
                this.closed ||
                socket.readyState !== WebSocket.OPEN
              )
                return;
              if (socket.bufferedAmount > 48_000) {
                this.fail(
                  "Dictation connection is too slow. Review the captured text.",
                );
                return;
              }
              socket.send(bytes);
            })
              .then(() => {
                if (!this.finishing && !this.closed)
                  this.callbacks.state("capturing");
              })
              .catch(() => {
                // Stop closes the encoder even when its worklet is still
                // starting. Keep the socket draining final transcripts.
                if (!this.finishing && !this.closed)
                  this.fail(
                    "Audio capture is unavailable. Use text or try another browser.",
                  );
              });
          } else if (event.type === "delta" || event.type === "completed") {
            const previous = this.items.get(event.item_id);
            if (!previous?.complete)
              this.items.set(event.item_id, {
                text:
                  event.type === "completed"
                    ? event.text
                    : (previous?.text ?? "") + event.text,
                complete: event.type === "completed",
                order: event.order,
              });
            if (
              this.items.size > 128 ||
              [...this.items.values()].reduce(
                (sum, item) => sum + item.text.length,
                0,
              ) > 32_000
            )
              throw new Error("Transcript limit");
            const parts = [...this.items.values()].sort(
              (a, b) => a.order - b.order,
            );
            this.callbacks.text({
              parts,
              final: [...this.items.values()]
                .sort((a, b) => a.order - b.order)
                .filter((item) => item.complete)
                .map((item) => item.text)
                .join(" "),
              interim: [...this.items.values()]
                .sort((a, b) => a.order - b.order)
                .filter((item) => !item.complete)
                .map((item) => item.text)
                .join(" "),
            });
          } else if (event.type === "error") this.fail(event.error.message);
          else if (event.type === "stopped") {
            this.dispose();
            this.callbacks.state("review");
          }
        } catch {
          this.fail(
            "Dictation state could not be verified. Review the captured text.",
          );
        }
      };
      socket.onclose = () => {
        if (!this.closed)
          this.fail("Dictation disconnected. Review the captured text.");
      };
      socket.onerror = () =>
        this.fail(
          "Dictation is unavailable. Check your connection and billing settings.",
        );
    } catch (error) {
      if (this.finishing || this.closed) return;
      this.fail(
        microphoneReady
          ? "Dictation options could not be prepared. Check your source and billing settings."
          : microphoneError(error),
      );
    }
  }
  stop() {
    if (this.finishing || this.closed) return;
    this.finishing = true;
    this.releaseMedia();
    if (!this.socket) {
      this.dispose();
      this.callbacks.state("review");
      return;
    }
    this.callbacks.state("finalizing");
    if (this.ready) this.requestStop();
  }
  private requestStop() {
    if (this.socket?.readyState === WebSocket.OPEN)
      this.socket.send('{"type":"stop"}');
    clearTimeout(this.timeout);
    this.timeout = setTimeout(
      () =>
        this.fail(
          "Final transcription was not confirmed. Review the text already received.",
        ),
      15_000,
    );
  }
  private fail(message: string) {
    if (this.closed) return;
    this.callbacks.error(message);
    this.dispose();
    this.callbacks.state("review");
  }
  private releaseMedia() {
    this.capture?.close();
    this.pcm?.close();
    this.meter?.close();
  }
  private dispose() {
    this.closed = true;
    clearTimeout(this.timeout);
    this.releaseMedia();
    this.socket?.close();
  }
  /** UI cancellation discards text immediately; provider finalization continues boundedly. */
  cancel() {
    this.stop();
  }
}
