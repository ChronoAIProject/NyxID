import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n";
import { ChatMessageList } from "./chat-message";
import { DecisionContinuation } from "./decision-continuation";
import type { ChatSessionState } from "@/lib/assistant/chat-types";

afterEach(async () => {
  cleanup();
  await appI18n.changeLanguage("en");
});

it("refreshes a continuation inside a memoized transcript row without rebuilding the projection", async () => {
  await appI18n.changeLanguage("en");
  const session: ChatSessionState = {
    clientId: "locale-test",
    conversationId: "nyxa-locale-test",
    title: "Thread",
    status: "completed_text",
    expectedTurnCount: 0,
    messages: [
      {
        id: "continuation",
        role: "user",
        content: "Confirmed",
        timestamp: 1,
        status: "complete",
        continuation_of: "card-1",
      },
    ],
  };
  const renderMessage = vi.fn(() => <DecisionContinuation />);
  render(
    <ChatMessageList
      session={session}
      bottomInset={0}
      renderMessage={renderMessage}
      projectionVersion="unchanged"
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent(
    "Continuing after your decision",
  );
  const initialRenders = renderMessage.mock.calls.length;

  for (const [language, label] of [
    ["zh-CN", "在你做出决定后继续"],
    ["zh-TW", "在你做出決定後繼續"],
  ]) {
    await act(async () => {
      await appI18n.changeLanguage(language);
    });
    expect(screen.getByRole("status")).toHaveTextContent(label!);
    expect(renderMessage).toHaveBeenCalledTimes(initialRenders);
  }
});
