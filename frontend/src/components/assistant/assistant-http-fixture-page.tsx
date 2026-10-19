import { useRouterState } from "@tanstack/react-router";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { assistantChatSurface } from "@/lib/assistant/conversation-ids";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { AssistantChatPage, NyxAgentAssistantChatPage, DirectAssistantChatPage } from "@/components/assistant/assistant-chat-page";
import { installAssistantHttpFixtures } from "@/lib/assistant/assistant-http-fixtures";
import type { ReactNode } from "react";

installAssistantHttpFixtures();

export function AssistantHttpFixturePage() {
  const search = useRouterState({ select: (state) => parseAssistantSearch(state.location.search as Record<string, unknown>) });
  const surface = assistantChatSurface({ nyxagentEnabled: useFeature(FEATURE_FLAG.NYXAGENT_ENGINE), directEnabled: useFeature(FEATURE_FLAG.DIRECT_CHAT_ENGINE), drafting: search.draft === true, selectedConversationId: search.c });
  return surface === "nyxagent" ? <NyxAgentAssistantChatPage /> : surface === "direct" ? <DirectAssistantChatPage /> : <AssistantChatPage />;
}

export function AssistantHttpFixtureBoundary({
  children,
}: {
  readonly children: ReactNode;
}) {
  return children;
}
