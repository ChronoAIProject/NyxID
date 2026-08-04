export type ContextMessage = {
  role: "user" | "assistant";
  content: string;
};

export type PendingAction = {
  action: string;
  collected_params: Record<string, unknown>;
  missing_params: string[];
  awaiting_confirmation: boolean;
};

export type SecretInputRequest = {
  param_name: string;
  label: string;
  description: string;
  placeholder: string;
};

export type SecretInput = {
  param_name: string;
  value: string;
};

export type ActionResult = {
  endpoint: string;
  status: number;
  summary: string;
};

export type ChatRequest = {
  message: string;
  context: ContextMessage[];
  pending_action?: PendingAction;
  secret_input?: SecretInput[];
};

export type ChatResponse = {
  reply: string;
  intent: string;
  intent_type: string;
  context_summary?: string;
  pending_action?: PendingAction;
  requires_secret_input?: SecretInputRequest[];
  action_result?: ActionResult;
};
