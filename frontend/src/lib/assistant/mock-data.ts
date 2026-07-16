import { applyTurnEvent } from "@/lib/assistant/stream";
import type {
  ApprovalCardContentBlock,
  AssistantMessage,
  ContentBlock,
  Conversation,
  ConversationHistory,
  RunContentBlock,
  TurnEvent,
  TurnReducerState,
} from "@/types/assistant";

const MINUTE = 60_000;
const DAY = 24 * 60 * MINUTE;

function clone<T>(value: T): T {
  return structuredClone(value);
}

function timestamp(value: number): string {
  return new Date(value).toISOString();
}

function textBlock(blockId: string, text: string): ContentBlock {
  return { type: "text", block_id: blockId, text };
}

interface StoredConversation {
  conversation: Conversation;
  turnState: TurnReducerState;
}

/**
 * View-model row for the Approvals workspace view. `requestedAt` is the
 * created_at of the message carrying the card — the PRD §5.5 block itself has
 * no decided_at, so the message timestamp is the honest "when" we can show.
 */
export interface ApprovalListEntry {
  readonly conversationId: string;
  readonly conversationTitle: string;
  readonly requestedAt: string;
  readonly block: ApprovalCardContentBlock;
}

function buildInitialConversations(now: number): StoredConversation[] {
  const showcaseCreated = timestamp(now - 3 * DAY);
  const showcaseUpdated = timestamp(now - 18 * MINUTE);
  const approvalExpires = timestamp(now + 14 * MINUTE);

  const approval: ApprovalCardContentBlock = {
    type: "approval_card",
    block_id: "approval-stripe-lark",
    approval_request_id: "approval-request-lark-1",
    body: "Post the drafted summary to #payments-oncall as the assistant bot.",
    service_slug: "lark-bot",
    agent_key_prefix: "nyxid_ag_...7f3d",
    approval_mode: "per_request",
    grant_duration_sec: null,
    expires_at: approvalExpires,
    decision: null,
    decision_channel: null,
  };

  const run: RunContentBlock = {
    type: "run",
    block_id: "run-stripe-digest",
    title: "RUN",
    steps_total: 3,
    steps_complete: 2,
    state: "awaiting_approval",
    steps: [
      {
        index: 1,
        status: "done",
        label: "stripe - GET /v1/charges?status=failed - 23 records",
        meta: "via credential broker - your API key was never exposed to the agent",
        service_slug: "stripe",
        artifact_id: null,
        approval_request_id: null,
      },
      {
        index: 2,
        status: "done",
        label: "Drafted summary - failed-payments-2026-07-13.md",
        meta: "artifact saved to this conversation",
        service_slug: null,
        artifact_id: "artifact-failed-payments",
        approval_request_id: null,
      },
      {
        index: 3,
        status: "waiting",
        label: "lark-bot - send message to #payments-oncall",
        meta: "waiting for approval - write action on a connected service",
        service_slug: "lark-bot",
        artifact_id: null,
        approval_request_id: "approval-request-lark-1",
      },
    ],
  };

  const showcaseMessages: AssistantMessage[] = [
    {
      id: "message-stripe-user",
      role: "user",
      schema_version: 1,
      created_at: showcaseCreated,
      blocks: [
        textBlock(
          "text-stripe-request",
          "Pull yesterday's failed Stripe payments, draft a short summary, and post it to **#payments-oncall** on Lark.",
        ),
      ],
    },
    {
      id: "message-stripe-connect",
      role: "assistant",
      schema_version: 1,
      created_at: timestamp(now - 2 * DAY),
      blocks: [
        textBlock(
          "text-stripe-connect-intro",
          "Before I can pull that, I need access to your **Stripe** account. The connection is now ready and scoped for this task.",
        ),
        {
          type: "connect_card",
          block_id: "connect-stripe",
          catalog_slug: "stripe",
          service_name: "Stripe",
          icon_url: "https://cdn.nyxid.dev/catalog/stripe.svg",
          subtitle: "Read access - charges & payments",
          auth_kind: "oauth",
          requested_scopes: ["charges:read"],
          key_id: "key-stripe-read",
          granted_scopes: ["charges:read"],
          device_user_code: null,
          device_verification_url: null,
          state: "connected",
          error_message: null,
          steps: [
            {
              title: "Authorize NyxID with Stripe",
              body: "Approve read-only access to charges. No funds can be moved.",
              done: true,
            },
            {
              title: "NyxID seals and brokers the credential",
              body: "The agent never receives the underlying Stripe credential.",
              done: true,
            },
            {
              title: "Continue automatically",
              body: "The task resumed as soon as NyxID confirmed the connection.",
              done: true,
            },
          ],
          footer: "Brokered by NyxID - read-only - revoke anytime in Studio",
        },
      ],
    },
    {
      id: "message-stripe-connected",
      role: "assistant",
      schema_version: 1,
      created_at: timestamp(now - DAY),
      blocks: [
        textBlock(
          "text-stripe-connected",
          "Stripe connected - `charges:read` granted - credential sealed in NyxID's vault.",
        ),
      ],
    },
    {
      id: "message-stripe-result",
      role: "assistant",
      schema_version: 1,
      created_at: showcaseUpdated,
      blocks: [
        textBlock(
          "text-stripe-result",
          "I found **23 failed payments** from yesterday totalling **$4,812.40** - mostly card declines. The summary is drafted; one write step needs your approval.",
        ),
        run,
        {
          type: "artifact",
          block_id: "artifact-block-failed-payments",
          artifact_id: "artifact-failed-payments",
          name: "failed-payments-2026-07-13.md",
          mime: "text/markdown",
          size_bytes: 2194,
          preview:
            "23 failed payments - $4,812.40\n17 card declines, 4 expired cards, and 2 disputes.",
          download_url: "/api/v1/assistant/artifacts/artifact-failed-payments",
        },
        approval,
      ],
    },
  ];

  const githubCreated = timestamp(now - 8 * DAY);
  const githubApprovalAt = timestamp(now - 2 * DAY - 12 * MINUTE);
  const githubUpdated = timestamp(now - 2 * DAY);
  const githubMessages: AssistantMessage[] = [
    {
      id: "message-github-user",
      role: "user",
      schema_version: 1,
      created_at: githubCreated,
      blocks: [
        textBlock(
          "text-github-user",
          "Rotate the deploy key for the web repository and verify access.",
        ),
      ],
    },
    {
      id: "message-github-approval",
      role: "assistant",
      schema_version: 1,
      created_at: githubApprovalAt,
      blocks: [
        textBlock(
          "text-github-approval-intro",
          "Rotating a deploy key is a write action on **GitHub**, so it needs your sign-off first.",
        ),
        {
          type: "approval_card",
          block_id: "approval-github-rotate",
          approval_request_id: "approval-request-github-1",
          body: "Rotate the deploy key for acme/web and revoke the previous key.",
          service_slug: "github",
          agent_key_prefix: "nyxid_ag_...7f3d",
          approval_mode: "per_request",
          grant_duration_sec: null,
          expires_at: timestamp(now - 2 * DAY + 3 * MINUTE),
          decision: "approved",
          decision_channel: "web",
        },
      ],
    },
    {
      id: "message-github-run",
      role: "assistant",
      schema_version: 1,
      created_at: githubUpdated,
      blocks: [
        textBlock(
          "text-github-intro",
          "The replacement key was created and verified without exposing it to the agent.",
        ),
        {
          type: "run",
          block_id: "run-github-key",
          title: "RUN",
          steps_total: 2,
          steps_complete: 2,
          state: "completed",
          steps: [
            {
              index: 1,
              status: "done",
              label: "github - rotate deploy key for acme/web",
              meta: "approved - write completed through the credential broker",
              service_slug: "github",
              artifact_id: null,
              approval_request_id: "approval-request-github-1",
            },
            {
              index: 2,
              status: "done",
              label: "Verify repository access",
              meta: "read check returned successfully",
              service_slug: "github",
              artifact_id: null,
              approval_request_id: null,
            },
          ],
        },
        textBlock(
          "text-github-complete",
          "Deploy access is healthy. The previous key is no longer active.",
        ),
      ],
    },
  ];

  const weeklyCreated = timestamp(now - 14 * DAY);
  const weeklyUpdated = timestamp(now - 7 * DAY);
  const weeklyShared = timestamp(now - 7 * DAY + 8 * MINUTE);
  const weeklyMessages: AssistantMessage[] = [
    {
      id: "message-weekly-user",
      role: "user",
      schema_version: 1,
      created_at: weeklyCreated,
      blocks: [
        textBlock(
          "text-weekly-user",
          "Prepare last week's NyxID usage report.",
        ),
      ],
    },
    {
      id: "message-weekly-result",
      role: "assistant",
      schema_version: 1,
      created_at: weeklyUpdated,
      blocks: [
        textBlock(
          "text-weekly-intro",
          "The report is ready. Agent traffic stayed within configured limits.",
        ),
        {
          type: "artifact",
          block_id: "artifact-block-weekly",
          artifact_id: "artifact-weekly",
          name: "weekly-usage-report-2026-W28.md",
          mime: "text/markdown",
          size_bytes: 1672,
          preview:
            "Agent runs: 184\nSuccessful runs: 176\nRuns requiring approval: 31",
          download_url: "/api/v1/assistant/artifacts/artifact-weekly",
        },
        textBlock(
          "text-weekly-complete",
          "I highlighted the eight failed runs and the services involved in the appendix.",
        ),
      ],
    },
    {
      id: "message-weekly-share-user",
      role: "user",
      schema_version: 1,
      created_at: timestamp(now - 7 * DAY + 4 * MINUTE),
      blocks: [
        textBlock(
          "text-weekly-share-user",
          "Post the summary to #leadership on Lark and email a copy to the external auditors.",
        ),
      ],
    },
    {
      id: "message-weekly-share",
      role: "assistant",
      schema_version: 1,
      created_at: weeklyShared,
      blocks: [
        textBlock(
          "text-weekly-share-intro",
          "Both shares are write actions gated by your policy, so each needs its own approval.",
        ),
        {
          type: "approval_card",
          block_id: "approval-weekly-lark",
          approval_request_id: "approval-request-weekly-lark",
          body: "Post the weekly usage summary to #leadership as the assistant bot.",
          service_slug: "lark-bot",
          agent_key_prefix: "nyxid_ag_...7f3d",
          approval_mode: "per_request",
          grant_duration_sec: null,
          expires_at: timestamp(now - 7 * DAY + 23 * MINUTE),
          decision: "expired",
          decision_channel: null,
        },
        {
          type: "approval_card",
          block_id: "approval-weekly-email",
          approval_request_id: "approval-request-weekly-email",
          body: "Email the full usage report to the external auditors' mailbox.",
          service_slug: "gmail",
          agent_key_prefix: "nyxid_ag_...7f3d",
          approval_mode: "per_request",
          grant_duration_sec: null,
          expires_at: timestamp(now - 7 * DAY + 23 * MINUTE),
          decision: "denied",
          decision_channel: "mobile",
        },
      ],
    },
  ];

  return [
    {
      conversation: {
        id: "conversation-stripe",
        title: "Failed Stripe payments digest",
        created_at: showcaseCreated,
        last_message_at: showcaseUpdated,
      },
      turnState: {
        messages: showcaseMessages,
        activeTurn: null,
        lastCursor: 0,
      },
    },
    {
      conversation: {
        id: "conversation-github",
        title: "Rotate GitHub deploy key",
        created_at: githubCreated,
        last_message_at: githubUpdated,
      },
      turnState: { messages: githubMessages, activeTurn: null, lastCursor: 0 },
    },
    {
      conversation: {
        id: "conversation-weekly",
        title: "Weekly usage report",
        created_at: weeklyCreated,
        last_message_at: weeklyShared,
      },
      turnState: { messages: weeklyMessages, activeTurn: null, lastCursor: 0 },
    },
  ];
}

export class MockAssistantStore {
  private readonly conversations = new Map<string, StoredConversation>();
  private now: () => number = Date.now;
  private localCounter = 0;
  private idCounter = 0;

  constructor() {
    this.reset();
  }

  reset(now: () => number = Date.now): void {
    this.now = now;
    this.localCounter = 0;
    this.idCounter = 0;
    this.conversations.clear();
    for (const item of buildInitialConversations(this.now())) {
      this.conversations.set(item.conversation.id, item);
    }
  }

  nextId(prefix: string): string {
    this.idCounter += 1;
    return `${prefix}-${String(this.idCounter)}`;
  }

  listConversations(): Conversation[] {
    return [...this.conversations.values()]
      .map((item) => clone(item.conversation))
      .sort((a, b) => b.last_message_at.localeCompare(a.last_message_at));
  }

  createConversation(): Conversation {
    this.localCounter += 1;
    const id = `local-${String(this.localCounter)}`;
    const createdAt = timestamp(this.now());
    const conversation: Conversation = {
      id,
      title: "New chat",
      created_at: createdAt,
      last_message_at: createdAt,
    };
    this.conversations.set(id, {
      conversation,
      turnState: { messages: [], activeTurn: null, lastCursor: 0 },
    });
    return clone(conversation);
  }

  getHistory(conversationId: string): ConversationHistory {
    const stored = this.requireConversation(conversationId);
    return clone({
      conversation: stored.conversation,
      messages: stored.turnState.messages,
      has_more: false,
    });
  }

  getTurnState(conversationId: string): TurnReducerState {
    return clone(this.requireConversation(conversationId).turnState);
  }

  appendUserMessage(conversationId: string, content: string): void {
    const normalized = content.trim();
    if (!normalized || normalized.length > 32_768) {
      throw new Error("Message must contain between 1 and 32768 characters.");
    }
    const stored = this.requireConversation(conversationId);
    const createdAt = timestamp(this.now());
    const message: AssistantMessage = {
      id: this.nextId("user-message"),
      role: "user",
      schema_version: 1,
      blocks: [textBlock(this.nextId("user-block"), normalized)],
      created_at: createdAt,
    };
    const firstMessage = stored.turnState.messages.length === 0;
    stored.turnState = {
      ...stored.turnState,
      messages: [...stored.turnState.messages, message],
      lastCursor: 0,
    };
    stored.conversation = {
      ...stored.conversation,
      title: firstMessage ? normalized.slice(0, 40) : stored.conversation.title,
      last_message_at: createdAt,
    };
  }

  applyEvent(conversationId: string, event: TurnEvent): void {
    const stored = this.requireConversation(conversationId);
    stored.turnState = applyTurnEvent(
      stored.turnState,
      event,
      timestamp(this.now()),
    );
    stored.conversation = {
      ...stored.conversation,
      last_message_at: timestamp(this.now()),
    };
  }

  findBlock(conversationId: string, blockId: string): ContentBlock | null {
    const messages =
      this.requireConversation(conversationId).turnState.messages;
    for (const message of messages) {
      const block = message.blocks.find(
        (candidate) => candidate.block_id === blockId,
      );
      if (block) return clone(block);
    }
    return null;
  }

  decideApproval(
    conversationId: string,
    blockId: string,
    approved: boolean,
  ): void {
    const stored = this.requireConversation(conversationId);
    const card = stored.turnState.messages
      .flatMap((message) => message.blocks)
      .find(
        (block): block is ApprovalCardContentBlock =>
          block.type === "approval_card" && block.block_id === blockId,
      );
    if (!card) throw new Error("Approval request was not found.");
    if (card.decision !== null) {
      throw new Error("Approval request was already decided.");
    }
    const requestId = card.approval_request_id;
    const isTerminalRun = (state: RunContentBlock["state"]) =>
      state === "completed" || state === "failed" || state === "cancelled";
    const isOpenStep = (status: RunContentBlock["steps"][number]["status"]) =>
      status === "waiting" || status === "active";

    // Denying a gate fails its run: the run's other open steps never execute,
    // so their still-pending approval requests are cancelled with them.
    const cancelledRequestIds = new Set<string>();
    if (!approved) {
      for (const message of stored.turnState.messages) {
        for (const block of message.blocks) {
          if (block.type !== "run" || isTerminalRun(block.state)) continue;
          if (
            !block.steps.some(
              (step) => step.approval_request_id === requestId,
            )
          ) {
            continue;
          }
          for (const step of block.steps) {
            if (
              step.approval_request_id !== null &&
              step.approval_request_id !== requestId &&
              isOpenStep(step.status)
            ) {
              cancelledRequestIds.add(step.approval_request_id);
            }
          }
        }
      }
    }

    const messages = stored.turnState.messages.map((message) => ({
      ...message,
      blocks: message.blocks.map((block): ContentBlock => {
        if (block.type === "approval_card") {
          if (block.block_id === blockId) {
            return {
              ...block,
              decision: approved ? "approved" : "denied",
              decision_channel: "web",
            };
          }
          if (
            block.decision === null &&
            cancelledRequestIds.has(block.approval_request_id)
          ) {
            return { ...block, decision: "cancelled", decision_channel: null };
          }
          return block;
        }
        if (block.type === "run") {
          // Only the non-terminal run gated by THIS approval request changes
          // state; failed/cancelled/completed runs are terminal forever.
          const referencesDecision = block.steps.some(
            (step) => step.approval_request_id === requestId,
          );
          if (!referencesDecision || isTerminalRun(block.state)) return block;
          const steps = block.steps.map((step) => {
            if (step.approval_request_id === requestId) {
              return {
                ...step,
                status: approved ? ("done" as const) : ("failed" as const),
                meta: approved
                  ? "approved - write completed through the credential broker"
                  : "denied - the write was not performed",
              };
            }
            if (!approved && isOpenStep(step.status)) {
              return {
                ...step,
                status: "skipped" as const,
                meta: "skipped - the run stopped after a denied approval",
              };
            }
            return step;
          });
          // Derive run state from the resulting steps: another gated step in
          // the same run may still be waiting on its own approval.
          const stillOpen = steps.some((step) => isOpenStep(step.status));
          return {
            ...block,
            steps,
            steps_complete: steps.filter((step) => step.status === "done")
              .length,
            state: !approved ? "failed" : stillOpen ? block.state : "completed",
          };
        }
        return block;
      }),
    }));
    stored.turnState = { ...stored.turnState, messages };
    stored.conversation = {
      ...stored.conversation,
      last_message_at: timestamp(this.now()),
    };
  }

  listApprovals(): ApprovalListEntry[] {
    const entries: ApprovalListEntry[] = [];
    for (const stored of this.conversations.values()) {
      for (const message of stored.turnState.messages) {
        for (const block of message.blocks) {
          if (block.type === "approval_card") {
            entries.push({
              conversationId: stored.conversation.id,
              conversationTitle: stored.conversation.title,
              requestedAt: message.created_at,
              block: clone(block),
            });
          }
        }
      }
    }
    // Pending first, then most recently requested.
    return entries.sort((a, b) => {
      const pendingDelta =
        Number(b.block.decision === null) - Number(a.block.decision === null);
      if (pendingDelta !== 0) return pendingDelta;
      return b.requestedAt.localeCompare(a.requestedAt);
    });
  }

  workspaceCounts(): { artifacts: number; pendingApprovals: number } {
    let artifacts = 0;
    let pendingApprovals = 0;
    for (const stored of this.conversations.values()) {
      for (const message of stored.turnState.messages) {
        for (const block of message.blocks) {
          if (block.type === "artifact") artifacts += 1;
          if (block.type === "approval_card" && block.decision === null) {
            pendingApprovals += 1;
          }
        }
      }
    }
    return { artifacts, pendingApprovals };
  }

  private requireConversation(conversationId: string): StoredConversation {
    const stored = this.conversations.get(conversationId);
    if (!stored) throw new Error("Conversation was not found.");
    return stored;
  }
}

export const assistantMockStore = new MockAssistantStore();

export function resetAssistantMockStore(now: () => number = Date.now): void {
  assistantMockStore.reset(now);
}

export function createScriptedTurn(
  turnId: string,
  messageId: string,
  blockId: string,
): TurnEvent[] {
  const chunks = [
    "I checked the current conversation context. ",
    "The next action will stay behind NyxID's credential broker, ",
    "use only the service scopes already granted, ",
    "and remain visible in the audit trail. ",
    "This mock turn is ready for the API transport swap.",
  ];
  const finalBlock: ContentBlock = {
    type: "text",
    block_id: blockId,
    text: chunks.join(""),
  };

  return [
    { cursor: 1, event: "turn.status", turn_id: turnId, status: "running" },
    {
      cursor: 2,
      event: "message.started",
      message_id: messageId,
      role: "assistant",
    },
    {
      cursor: 3,
      event: "block.started",
      message_id: messageId,
      block_id: blockId,
      index: 0,
      block: { type: "text", block_id: blockId, text: "" },
    },
    ...chunks.map<TurnEvent>((chunk, index) => ({
      cursor: index + 4,
      event: "block.delta",
      block_id: blockId,
      text: chunk,
    })),
    {
      cursor: 9,
      event: "block.completed",
      block_id: blockId,
      block: finalBlock,
    },
    { cursor: 10, event: "message.completed", message_id: messageId },
    {
      cursor: 11,
      event: "turn.completed",
      turn_id: turnId,
      status: "completed",
      error: null,
    },
  ];
}
