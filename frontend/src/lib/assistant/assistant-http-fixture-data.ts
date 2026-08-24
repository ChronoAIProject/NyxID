import type {
  ConversationMeta,
  StoredChatMessage,
} from "@/lib/assistant/chat-types";

export const FIXTURE_REPLY =
  "I checked the current conversation context. The next action will stay behind NyxID's credential broker, use only the service scopes already granted, and remain visible in the audit trail. This mock turn is ready for the API transport swap.";
export const FIXTURE_TOOL_RESULT = "Posted to #payments-oncall";

export interface AssistantFixtureConversation {
  meta: ConversationMeta;
  messages: StoredChatMessage[];
  pendingApproval: Record<string, unknown> | null;
  latestApprovalResolution: Record<string, unknown> | null;
  stateVersion: number;
  progressSequence: number;
  activeTurn: Record<string, unknown> | null;
  latestTurn: Record<string, unknown> | null;
  activeTask: Record<string, unknown> | null;
  pendingInput: Record<string, unknown> | null;
  pendingActions: Record<string, unknown>[];
  recentActions: Record<string, unknown>[];
}

function storedMessage(
  id: string,
  role: "user" | "assistant",
  content: string,
  timestamp: number,
): StoredChatMessage {
  return {
    id,
    role,
    content,
    timestamp,
    status: "completed",
    turnId: `${id}-turn`,
  };
}

function conversation(
  id: string,
  title: string,
  messages: StoredChatMessage[],
  ageDays: number,
  pendingApproval: Record<string, unknown> | null = null,
): AssistantFixtureConversation {
  const now = Date.now();
  const createdAt = new Date(now - ageDays * 86_400_000).toISOString();
  const updatedAt = new Date(
    messages.at(-1)?.timestamp ?? Date.parse(createdAt),
  ).toISOString();
  return {
    meta: {
      id,
      title,
      createdAt,
      updatedAt,
      messageCount: messages.length,
      stateVersion: 3,
      taskStatus: null,
      attentionKind: pendingApproval ? "approval" : null,
      attentionSince: pendingApproval ? updatedAt : null,
      activeStepSummary: null,
    },
    messages,
    pendingApproval,
    latestApprovalResolution: null,
    stateVersion: 3,
    progressSequence: 3,
    activeTurn: null,
    latestTurn: {
      turnId: `${id}-settled-turn`,
      status: "completed",
    },
    activeTask: null,
    pendingInput: null,
    pendingActions: [],
    recentActions: [],
  };
}

export function createSeededAssistantFixtureConversations(): AssistantFixtureConversation[] {
  const now = Date.now();
  return [
    conversation(
      "conversation-stripe",
      "Failed Stripe payments digest",
      [
        storedMessage(
          "message-stripe-user",
          "user",
          "Pull yesterday's failed Stripe payments, draft a short summary, and post it to #payments-oncall on Lark.",
          now - 3 * 86_400_000,
        ),
        storedMessage(
          "message-stripe-connect",
          "assistant",
          "Before I can pull that, I need access to your Stripe account. The connection is now ready and scoped for this task.",
          now - 2 * 86_400_000,
        ),
        storedMessage(
          "message-stripe-connected",
          "assistant",
          "Stripe connected - charges:read granted - credential sealed in NyxID's vault.",
          now - 86_400_000,
        ),
        storedMessage(
          "message-stripe-result",
          "assistant",
          "I found 23 failed payments from yesterday totalling $4,812.40. The summary is drafted; one write step needs your approval.",
          now - 18 * 60_000,
        ),
      ],
      3,
      {
        approvalRequestId: "approval-request-lark-1",
        toolName: "Post the drafted summary to #payments-oncall",
        action: "lark.postMessage",
      },
    ),
    conversation(
      "conversation-github",
      "Rotate GitHub deploy key",
      [
        storedMessage(
          "message-github-user",
          "user",
          "Rotate the deploy key for the web repository and verify access.",
          now - 8 * 86_400_000,
        ),
        storedMessage(
          "message-github-approval",
          "assistant",
          "Rotating a deploy key is a write action on GitHub, so it needs your sign-off first. Approved through NyxID.",
          now - 2 * 86_400_000 - 12 * 60_000,
        ),
        storedMessage(
          "message-github-run",
          "assistant",
          "The replacement key was created and verified without exposing it to the agent. Deploy access is healthy. The previous key is no longer active.",
          now - 2 * 86_400_000,
        ),
      ],
      8,
    ),
    conversation(
      "conversation-weekly",
      "Weekly usage report",
      [
        storedMessage(
          "message-weekly-user",
          "user",
          "Prepare last week's NyxID usage report.",
          now - 14 * 86_400_000,
        ),
        storedMessage(
          "message-weekly-result",
          "assistant",
          "The report is ready. Agent traffic stayed within configured limits. I highlighted the eight failed runs in the appendix.",
          now - 7 * 86_400_000,
        ),
        storedMessage(
          "message-weekly-share-user",
          "user",
          "Post the summary to #leadership on Lark and email a copy to the external auditors.",
          now - 7 * 86_400_000 + 4 * 60_000,
        ),
        storedMessage(
          "message-weekly-share",
          "assistant",
          "Both shares are write actions gated by your policy, so each needs its own approval.",
          now - 7 * 86_400_000 + 8 * 60_000,
        ),
      ],
      14,
    ),
  ];
}

export function activeTaskFixture(actorId: string, turnId: string) {
  return {
    schemaVersion: 4,
    actorId,
    taskId: `task-${turnId}`,
    turnId,
    planId: `plan-${turnId}`,
    planRevision: 1,
    planRevisions: [],
    title: "Inspect connected services",
    status: "active",
    activeStepId: `step-${turnId}`,
    steps: [
      {
        stepId: `step-${turnId}`,
        order: 1,
        kind: "tool",
        status: "running",
        required: true,
        description: "Inspect connected services",
        source: {
          tool: { toolName: "lark.postMessage", serviceSlug: "lark-bot" },
        },
        mayChangeExternalState: false,
        externalEffect: "not_started",
        availableActions: { stop: true },
        dependsOn: [],
        substeps: [],
        operation: {
          conversationActorId: actorId,
          turnId,
          taskId: `task-${turnId}`,
          stepId: `step-${turnId}`,
          operationId: `operation-${turnId}`,
          operationGeneration: 1,
          phase: "running",
        },
      },
    ],
  };
}

export function assistantFixtureFrames(
  actorId: string,
  turnId: string,
  messageId: string,
  output = FIXTURE_REPLY,
): unknown[] {
  const task = activeTaskFixture(actorId, turnId);
  const step = task.steps[0]!;
  const actionRequest = {
    schemaVersion: 4,
    actorId,
    originTurnId: turnId,
    taskId: task.taskId,
    stepId: step.stepId,
    actionRequestId: `action-${turnId}`,
    action: "service.connect",
    params: {
      catalogService: {
        serviceSlug: "api-github",
        requestedScopes: ["repo"],
      },
    },
  };
  const splitAt = Math.max(1, Math.floor(output.length / 4));
  const chunks = [
    output.slice(0, splitAt),
    output.slice(splitAt, splitAt * 2),
    output.slice(splitAt * 2, splitAt * 3),
    output.slice(splitAt * 3),
  ].filter(Boolean);
  return [
    { runStarted: { actorId, runId: turnId, commandId: `command-${turnId}` } },
    {
      sequence: 91,
      custom: { name: "nyxid.task.snapshot", payload: task },
    },
    {
      custom: {
        name: "aevatar.llm.reasoning",
        payload: { role: "assistant", delta: "Checking current state." },
      },
    },
    { stepStarted: { stepName: "Inspect connected services" } },
    {
      toolCallStart: {
        toolCallId: `tool-${turnId}`,
        toolName: "lark.postMessage",
      },
    },
    { textMessageStart: { messageId, role: "assistant" } },
    ...chunks.map((delta) => ({ textMessageContent: { messageId, delta } })),
    {
      sequence: 92,
      custom: {
        name: "nyxid.task.step.changed",
        payload: { taskId: task.taskId, planRevision: 1, step },
      },
    },
    {
      sequence: 93,
      custom: {
        name: "nyxid.control.changed",
        payload: { requestId: `control-${turnId}`, outcome: "accepted" },
      },
    },
    {
      sequence: 94,
      custom: {
        name: "nyxid.step.control.changed",
        payload: { requestId: `step-control-${turnId}`, outcome: "accepted" },
      },
    },
    {
      sequence: 95,
      custom: {
        name: "nyxid.continuation.changed",
        payload: { originTurnId: turnId, outcome: "accepted" },
      },
    },
    {
      sequence: 96,
      custom: {
        name: "nyxid.input.request",
        payload: {
          requestId: `input-${turnId}`,
          prompt: "Confirm the mock region",
          options: [
            { optionId: "sg", label: "Singapore" },
            { optionId: "fra", label: "Frankfurt" },
          ],
          allowFreeText: false,
          multiSelect: false,
        },
      },
    },
    {
      sequence: 97,
      custom: {
        name: "nyxid.input.changed",
        payload: { requestId: `input-${turnId}`, outcome: "resolved" },
      },
    },
    {
      sequence: 98,
      custom: {
        name: "nyxid.approval.request",
        payload: {
          approvalRequestId: `approval-${turnId}`,
          toolName: "lark.postMessage",
        },
      },
    },
    {
      sequence: 99,
      custom: {
        name: "nyxid.approval.changed",
        payload: {
          approvalRequestId: `approval-${turnId}`,
          outcome: "approved",
        },
      },
    },
    {
      sequence: 100,
      custom: { name: "nyxid.action.request", payload: actionRequest },
    },
    {
      type: "MEDIA_CONTENT",
      mediaContent: {
        dataBase64: "SGVsbG8=",
        kind: "text",
        mediaType: "text/plain",
        name: "fixture.txt",
      },
    },
    {
      toolCallEnd: {
        toolCallId: `tool-${turnId}`,
        result: FIXTURE_TOOL_RESULT,
      },
    },
    { stepFinished: { stepName: "Inspect connected services", success: true } },
    { textMessageEnd: { messageId, message: output } },
    {
      runFinished: {
        actorId,
        runId: turnId,
        result: {
          output,
          usage: { totalTokens: 42, model: "mock-console-parity" },
        },
      },
    },
  ];
}
