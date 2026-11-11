import type { VoiceSnapshot } from "@/schemas/assistant-voice";
import type { ChatMessage } from "@/lib/assistant/chat-types";
export type VoiceCaption = VoiceSnapshot["captions"][number];
export type VoiceView = {
  panelId?: string;
  threadId: string;
  actorId?: string;
  snapshot: VoiceSnapshot | null;
  active: boolean;
  activity: import("@/lib/microphone-capture").AudioActivity;
};

/** Same durable segment ID replaces a live caption when history catches up. */
export function mergeVoiceCaptions(
  messages: readonly ChatMessage[],
  captions: readonly VoiceCaption[],
): ChatMessage[] {
  const ids = new Set(messages.map((message) => message.id));
  return [
    ...messages,
    ...captions
      .filter((caption) => !ids.has(caption.id))
      .map(
        (caption): ChatMessage => ({
          id: caption.id,
          role: caption.speaker,
          content: caption.text,
          status: caption.sealed ? "complete" : "streaming",
          timestamp: 0,
          via: "voice",
        }),
      ),
  ];
}

/** Changes only at a real fragment boundary or caption mutation, not every audio sample. */
export function voiceDeliveryRevision(
  captions: readonly VoiceCaption[],
  playbackMs: number,
): string {
  return JSON.stringify(
    captions
      .slice(-16)
      .map((c) => [
        c.id,
        c.text,
        c.sealed,
        c.complete,
        c.fragments
          ?.slice(0, 256)
          .map((f) => [f.text, f.start_ms, f.end_ms, playbackMs >= f.end_ms]),
        c.end_ms != null && playbackMs >= c.end_ms,
      ]),
  );
}
