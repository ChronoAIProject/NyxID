import type {
  RuntimeEvent,
  RuntimeRunInterventionInfo,
  RuntimeStepInfo,
  RuntimeToolApprovalRequestInfo,
  RuntimeToolCallInfo,
} from "@/lib/assistant/runtime-event-semantics";

export type { RuntimeEvent };

type ExtensibleString<T extends string> = T | (string & Record<never, never>);

export type ChatMessageRole = ExtensibleString<"user" | "assistant">;
export type ChatMessageStatus = ExtensibleString<
  "complete" | "streaming" | "error"
>;
export type StoredChatMessageStatus = ExtensibleString<"complete" | "error">;

export interface ChatMessage {
  readonly id: string;
  readonly role: ChatMessageRole;
  readonly content: string;
  readonly timestamp: number;
  readonly status: ChatMessageStatus;
  readonly turnId?: string | null;
  readonly authorId?: string | null;
  readonly authorName?: string | null;
  readonly error?: string | null;
  readonly events?: RuntimeEvent[];
  readonly pendingApproval?: RuntimeToolApprovalRequestInfo;
  readonly pendingRunIntervention?: RuntimeRunInterventionInfo;
  readonly steps?: RuntimeStepInfo[];
  readonly thinking?: string | null;
  readonly toolCalls?: RuntimeToolCallInfo[];
}

export interface ConversationRuntimeIdentity {
  readonly actorId?: string;
  readonly commandId?: string;
  readonly runId?: string;
}

export interface ConversationLlmPreferences {
  readonly llmModel?: string;
  readonly llmRoute?: string;
}

export interface ConversationSessionSnapshot {
  readonly preferences?: ConversationLlmPreferences;
  readonly runtime?: ConversationRuntimeIdentity;
}

export interface ConversationMeta {
  readonly id: string;
  readonly title: string;
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly messageCount: number;
  readonly serviceId?: string;
  readonly serviceKind?: string;
  readonly llmRoute?: string | null;
  readonly llmModel?: string | null;
  readonly stateVersion?: number;
  readonly taskStatus?: string | null;
  readonly attentionKind?: string | null;
  readonly attentionSince?: string | null;
  readonly activeStepSummary?: string | null;
}

export interface ConversationSessionMeta extends ConversationMeta {
  readonly actorId?: string;
  readonly commandId?: string;
  readonly runId?: string;
  readonly session?: ConversationSessionSnapshot;
}

export interface StoredChatMessage {
  readonly id: string;
  readonly turnId?: string | null;
  readonly role: ChatMessageRole;
  readonly content: string;
  readonly timestamp: number;
  readonly status: StoredChatMessageStatus;
  readonly error?: string | null;
  readonly authorId?: string | null;
  readonly authorName?: string | null;
  readonly thinking?: string | null;
}

export interface ChatConversationDetail {
  readonly messages: StoredChatMessage[];
  readonly stateVersion: number;
  readonly projectionStatus: "current" | "pending";
}

export interface ChatHistoryIndex {
  readonly conversations: ConversationMeta[];
  readonly nextCursor?: string | null;
}

export interface ChatSessionState {
  readonly conversationId?: string;
  readonly latestTurnId?: string;
  readonly messages: readonly ChatMessage[];
  readonly status:
    | "draft"
    | "streaming"
    | "completed_text"
    | "error"
    | "stopped";
}
