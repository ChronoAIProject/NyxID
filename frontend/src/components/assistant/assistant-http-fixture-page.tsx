import { AssistantChatPage } from "@/components/assistant/assistant-chat-page";
import { installAssistantHttpFixtures } from "@/lib/assistant/assistant-http-fixtures";

installAssistantHttpFixtures();

export function AssistantHttpFixturePage() {
  return <AssistantChatPage />;
}
