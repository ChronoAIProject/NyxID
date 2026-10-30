import { z } from "zod";
import type {
  VoiceControlSocket,
  VoiceMediaAdapter,
} from "./AssistantVoiceClient";
import {
  voicePreferencesSchema,
  type VoicePreferences,
} from "./voiceContracts";
import { assistantDictationPath } from "../../../lib/api/assistantVoiceTransport";

// Matches the web AssistantDictationClient protocol. Text stays in local review;
// there is deliberately no chat-send, history or conversational tool API here.
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
export type DictationPhase = "connecting" | "listening" | "stopping" | "review";
export type DictationText = { final: string; interim: string };

type Options = {
  media: VoiceMediaAdapter;
  createSocket(path: string, signal: AbortSignal): Promise<VoiceControlSocket>;
  randomId(): string;
  onText(text: DictationText): void;
  onPhase(phase: DictationPhase): void;
  onError(message: string): void;
};

/** One bounded transcription attempt, using the same initial JSON/PCM/stop as web. */
export class AssistantDictationClient {
  private controller = new AbortController();
  private socket?: VoiceControlSocket;
  private started = false;
  private closed = false;
  private finishing = false;
  private ready = false;
  private detached = false;
  private timer?: ReturnType<typeof setTimeout>;
  private limit?: ReturnType<typeof setTimeout>;
  private items = new Map<
    string,
    { text: string; complete: boolean; order: number }
  >();
  private unsubscribe: () => void;

  constructor(private options: Options) {
    this.unsubscribe = options.media.subscribe((event) => {
      if (this.closed || this.finishing) return;
      if (event.type === "error")
        this.fail("Dictation audio stopped. Review the text already captured.");
      if (event.type !== "pcm" || !this.ready) return;
      if (
        !event.bytes.byteLength ||
        event.bytes.byteLength > 4096 ||
        event.bytes.byteLength % 2
      )
        return this.fail("Dictation audio could not be verified.");
      if ((this.socket?.bufferedAmount ?? 0) > 48_000)
        return this.fail(
          "Dictation connection is too slow. Review the captured text.",
        );
      this.send(event.bytes);
    });
  }

  async start(thread: string, preferences: VoicePreferences) {
    if (this.started || this.closed) return;
    this.started = true;
    this.phase("connecting");
    try {
      const prepared = voicePreferencesSchema.parse(preferences);
      if (
        prepared.input_mode !== "push_to_talk" ||
        prepared.key_source !== "platform" ||
        prepared.connection_id !== null
      )
        throw new Error("Invalid dictation preferences");
      await this.options.media.prepare("dictation", this.controller.signal);
      if (this.closed) return;
      const socket = await this.options.createSocket(
        assistantDictationPath(thread),
        this.controller.signal,
      );
      if (this.closed) {
        socket.close();
        return;
      }
      this.socket = socket;
      socket.binaryType = "arraybuffer";
      let initialSent = false;
      const initialize = () => {
        if (this.closed || initialSent) return;
        initialSent = true;
        this.send(
          JSON.stringify({
            client_request_id: this.options.randomId(),
            preferences: prepared,
          }),
        );
      };
      socket.onopen = initialize;
      socket.onmessage = ({ data }) => this.receive(data);
      socket.onerror = () =>
        this.fail(
          "Dictation is unavailable. Check your connection and billing settings.",
        );
      socket.onclose = () => {
        if (!this.closed)
          this.fail(
            "Dictation disconnected. Review the text already received.",
          );
      };
      this.deadline("Dictation could not connect. Try again.");
      if (socket.readyState === 1) initialize();
    } catch {
      if (!this.closed)
        this.fail(
          "Dictation could not start. Check microphone access and your connection.",
        );
    }
  }

  private receive(raw: unknown) {
    if (this.closed) return;
    try {
      if (typeof raw !== "string" || raw.length > 262_144) throw new Error();
      const event = eventSchema.parse(JSON.parse(raw));
      if (event.type === "ready") {
        if (this.ready) throw new Error();
        this.ready = true;
        clearTimeout(this.timer);
        if (this.finishing) this.requestStop();
        else {
          this.options.media.setInput(true);
          this.phase("listening");
          this.limit = setTimeout(() => this.stop(), 120_000);
        }
      } else if (event.type === "delta" || event.type === "completed") {
        if (!this.ready) throw new Error();
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
        const parts = [...this.items.values()].sort(
          (a, b) => a.order - b.order,
        );
        if (
          parts.length > 128 ||
          parts.reduce((sum, item) => sum + item.text.length, 0) > 32_000
        )
          throw new Error();
        if (!this.detached)
          this.options.onText({
            final: parts
              .filter((p) => p.complete)
              .map((p) => p.text)
              .join(" "),
            interim: parts
              .filter((p) => !p.complete)
              .map((p) => p.text)
              .join(" "),
          });
      } else if (event.type === "stopped") {
        this.dispose();
        if (!event.confirmed && !this.detached)
          this.options.onError(
            "Final transcription was not confirmed. Review the text already received.",
          );
        this.phase("review");
      } else
        this.fail(
          "Dictation is unavailable. Check your connection and billing settings.",
        );
    } catch {
      this.fail(
        "Dictation state could not be verified. Review the text already received.",
      );
    }
  }

  private phase(phase: DictationPhase) {
    if (!this.detached) this.options.onPhase(phase);
  }
  private send(data: string | ArrayBuffer) {
    if (this.closed) return;
    try {
      if (this.socket?.readyState !== 1) throw new Error();
      this.socket.send(data);
    } catch {
      this.fail("Dictation connection lost. Review the captured text.");
    }
  }
  private deadline(message: string) {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.fail(message), 15_000);
  }
  stop() {
    if (this.finishing || this.closed) return;
    this.finishing = true;
    clearTimeout(this.limit);
    this.options.media.setInput(false);
    this.options.media.close();
    if (!this.socket) {
      this.dispose();
      this.phase("review");
      return;
    }
    this.phase("stopping");
    if (this.ready) this.requestStop();
  }
  private requestStop() {
    this.send('{"type":"stop"}');
    if (!this.closed)
      this.deadline(
        "Final transcription was not confirmed. Review the text already received.",
      );
  }
  /** Release capture immediately; allow bounded server finalization without UI writes. */
  cancel() {
    this.detached = true;
    this.stop();
  }
  private fail(message: string) {
    if (this.closed) return;
    if (!this.detached) this.options.onError(message);
    this.dispose();
    this.phase("review");
  }
  private dispose() {
    if (this.closed) return;
    this.closed = true;
    clearTimeout(this.timer);
    clearTimeout(this.limit);
    this.unsubscribe();
    this.controller.abort();
    this.options.media.close();
    this.socket?.close();
  }
}
