import { AssistantChatPage } from "@/components/assistant/assistant-chat-page";
import { installAssistantHttpFixtures } from "@/lib/assistant/assistant-http-fixtures";

export function AssistantHttpFixturePage() {
  installAssistantHttpFixtures();
  return <AssistantChatPage />;
}
