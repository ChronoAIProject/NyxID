import type { ChatRequest, ChatResponse } from "./chatTypes";
import { requestJson } from "./http";

export async function sendChatMessage(request: ChatRequest): Promise<ChatResponse> {
  return requestJson<ChatResponse>("/chat", {
    method: "POST",
    body: request,
  });
}
