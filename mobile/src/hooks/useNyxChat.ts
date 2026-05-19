import { useCallback, useRef, useState } from "react";
import { sendChatMessage } from "../lib/api/chatApi";
import type { ContextMessage, PendingAction, SecretInputRequest, SecretInput } from "../lib/api/chatTypes";

export type NyxMessage = {
  _id: string;
  text: string;
  createdAt: Date;
  user: { _id: string | number; name?: string };
};

const NYX_USER = { _id: "nyx", name: "Nyx" };
const HUMAN_USER = { _id: "user" };
const CONTEXT_WINDOW = 5;

let msgCounter = 0;
function nextId(): string {
  msgCounter += 1;
  return `msg-${msgCounter}-${Date.now()}`;
}

export function useNyxChat() {
  const [messages, setMessages] = useState<NyxMessage[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [pendingAction, setPendingAction] = useState<PendingAction | undefined>();
  const [requiresSecretInput, setRequiresSecretInput] = useState<SecretInputRequest[] | undefined>();
  const contextRef = useRef<ContextMessage[]>([]);

  const addBotMessage = useCallback((text: string) => {
    const msg: NyxMessage = { _id: nextId(), text, createdAt: new Date(), user: NYX_USER };
    setMessages((prev) => [msg, ...prev]);
  }, []);

  const sendMessage = useCallback(async (text: string) => {
    const userMsg: NyxMessage = { _id: nextId(), text, createdAt: new Date(), user: HUMAN_USER };
    setMessages((prev) => [userMsg, ...prev]);

    contextRef.current = [
      ...contextRef.current,
      { role: "user" as const, content: text },
    ].slice(-CONTEXT_WINDOW * 2);

    setIsLoading(true);
    try {
      const response = await sendChatMessage({
        message: text,
        context: contextRef.current,
        pending_action: pendingAction,
      });

      contextRef.current = [
        ...contextRef.current,
        { role: "assistant" as const, content: response.reply },
      ].slice(-CONTEXT_WINDOW * 2);

      addBotMessage(response.reply);
      setPendingAction(response.pending_action);
      setRequiresSecretInput(response.requires_secret_input);
    } catch {
      addBotMessage("Something went wrong. Please try again.");
    } finally {
      setIsLoading(false);
    }
  }, [pendingAction, addBotMessage]);

  const submitSecretInput = useCallback(async (inputs: SecretInput[]) => {
    setRequiresSecretInput(undefined);
    setIsLoading(true);
    try {
      const response = await sendChatMessage({
        message: "",
        context: contextRef.current,
        pending_action: pendingAction,
        secret_input: inputs,
      });
      addBotMessage(response.reply);
      setPendingAction(response.pending_action);
    } catch {
      addBotMessage("Failed to save credentials. Please try again.");
    } finally {
      setIsLoading(false);
    }
  }, [pendingAction, addBotMessage]);

  const clearChat = useCallback(() => {
    setMessages([]);
    contextRef.current = [];
    setPendingAction(undefined);
    setRequiresSecretInput(undefined);
  }, []);

  return {
    messages,
    isLoading,
    pendingAction,
    requiresSecretInput,
    sendMessage,
    submitSecretInput,
    clearChat,
  };
}
