import { AssistantChatPage } from "@/components/assistant/assistant-chat-page";
import { installAssistantHttpFixtures } from "@/lib/assistant/assistant-http-fixtures";
import type { ReactNode } from "react";

installAssistantHttpFixtures();

export function AssistantHttpFixturePage() {
  return <AssistantChatPage />;
}

export function AssistantHttpFixtureBoundary({
  children,
}: {
  readonly children: ReactNode;
}) {
  return children;
}
