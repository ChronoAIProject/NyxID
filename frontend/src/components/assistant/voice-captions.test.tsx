import { render } from "@testing-library/react";
import { expect, it } from "vitest";
import { ChatMessageList } from "./chat-message";
import { VoiceCaptionText } from "./voice-captions";
import {
  voiceDeliveryRevision,
  mergeVoiceCaptions,
  type VoiceCaption,
} from "./voice-caption-state";
import type { ChatSessionState } from "@/lib/assistant/chat-types";
it("memoized persisted message brightens only at real fragment boundaries", () => {
  const caption: VoiceCaption = {
    id: "persisted",
    speaker: "assistant",
    text: "One two",
    sealed: true,
    complete: true,
    end_ms: 400,
    fragments: [
      { text: "One ", start_ms: 0, end_ms: 200 },
      { text: "two", start_ms: 200, end_ms: 400 },
    ],
  };
  const message = {
    id: caption.id,
    role: "assistant" as const,
    content: caption.text,
    timestamp: 0,
    status: "complete" as const,
  };
  const session: ChatSessionState = {
    clientId: "test",
    conversationId: "thread",
    status: "completed_text",
    title: "test",
    messages: [message],
    expectedTurnCount: 0,
  };
  expect(mergeVoiceCaptions(session.messages, [caption])).toEqual([message]);
  const list = (at: number) => (
    <ChatMessageList
      session={session}
      bottomInset={0}
      projectionVersion={voiceDeliveryRevision([caption], at)}
      renderMessage={() => (
        <VoiceCaptionText caption={caption} playbackMs={at} />
      )}
    />
  );
  const { container, rerender } = render(list(0));
  expect(
    container.querySelectorAll('[data-voice-delivery="played"]'),
  ).toHaveLength(0);
  expect(voiceDeliveryRevision([caption], 20)).toBe(
    voiceDeliveryRevision([caption], 199),
  );
  rerender(list(200));
  expect(
    container.querySelectorAll('[data-voice-delivery="played"]'),
  ).toHaveLength(1);
  rerender(list(400));
  expect(
    container.querySelectorAll('[data-voice-delivery="played"]'),
  ).toHaveLength(2);
});
