import type {
  AssistantMessage,
  ContentBlock,
  TurnEvent,
  TurnReducerState,
} from "@/types/assistant";

export const EMPTY_TURN_STATE: TurnReducerState = {
  messages: [],
  activeTurn: null,
  lastCursor: 0,
};

/**
 * Snapshot a block into a terminal state for turn cancellation, per the PRD
 * §5.6 lifecycle rule: no card may be left permanently pending. Blocks already
 * in a terminal state (and immutable/text blocks) pass through unchanged.
 */
export function toTerminalBlock(block: ContentBlock): ContentBlock {
  switch (block.type) {
    case "action_card":
      // Browser actions outlive the turn that requested them. In particular,
      // cancel must not turn a still-actionable card into a zombie receipt.
      return block;
    case "run":
      if (
        block.state === "completed" ||
        block.state === "failed" ||
        block.state === "cancelled"
      ) {
        return block;
      }
      return {
        ...block,
        state: "cancelled",
        steps: block.steps.map((step) =>
          step.status === "active" || step.status === "waiting"
            ? { ...step, status: "skipped" as const }
            : step,
        ),
      };
    case "approval_card":
      return block.decision === null
        ? { ...block, decision: "cancelled" }
        : block;
    case "connect_card":
      if (
        block.state === "connected" ||
        block.state === "error" ||
        block.state === "timed_out"
      ) {
        return block;
      }
      return { ...block, state: "timed_out" };
    default:
      return block;
  }
}

function updateBlock(
  messages: AssistantMessage[],
  blockId: string,
  update: (block: ContentBlock) => ContentBlock,
): AssistantMessage[] {
  let changed = false;
  const next = messages.map((message) => {
    const index = message.blocks.findIndex(
      (block) => block.block_id === blockId,
    );
    if (index < 0) return message;
    const blocks = [...message.blocks];
    const current = blocks[index];
    if (!current) return message;
    blocks[index] = update(current);
    changed = true;
    return { ...message, blocks };
  });
  return changed ? next : messages;
}

function replaceBlock(
  messages: AssistantMessage[],
  blockId: string,
  replacement: ContentBlock,
): AssistantMessage[] {
  return updateBlock(messages, blockId, () => replacement);
}

export function applyTurnEvent(
  state: TurnReducerState,
  event: TurnEvent,
  receivedAt = new Date().toISOString(),
): TurnReducerState {
  if (event.cursor <= state.lastCursor) return state;

  const nextBase = { ...state, lastCursor: event.cursor };

  switch (event.event) {
    case "turn.status":
      return {
        ...nextBase,
        activeTurn: {
          turnId: event.turn_id,
          status: event.status,
          error: null,
        },
      };
    case "message.started": {
      if (state.messages.some((message) => message.id === event.message_id)) {
        return nextBase;
      }
      return {
        ...nextBase,
        messages: [
          ...state.messages,
          {
            id: event.message_id,
            role: event.role,
            schema_version: 1,
            blocks: [],
            created_at: receivedAt,
          },
        ],
      };
    }
    case "block.started": {
      const messages = state.messages.map((message) => {
        if (message.id !== event.message_id) return message;
        const blocks = [...message.blocks];
        const existingIndex = blocks.findIndex(
          (block) => block.block_id === event.block_id,
        );
        if (existingIndex >= 0) {
          blocks[existingIndex] = event.block;
        } else {
          blocks.splice(Math.min(event.index, blocks.length), 0, event.block);
        }
        return { ...message, blocks };
      });
      return { ...nextBase, messages };
    }
    case "block.delta":
      return {
        ...nextBase,
        messages: updateBlock(state.messages, event.block_id, (block) =>
          block.type === "text"
            ? { ...block, text: block.text + event.text }
            : block,
        ),
      };
    case "block.updated":
      return {
        ...nextBase,
        messages: updateBlock(
          state.messages,
          event.block_id,
          (block) =>
            ({
              ...block,
              ...event.patch,
              type: block.type,
              block_id: block.block_id,
            }) as ContentBlock,
        ),
      };
    case "block.completed":
      return {
        ...nextBase,
        messages: replaceBlock(state.messages, event.block_id, event.block),
      };
    case "message.completed":
      return nextBase;
    case "turn.completed":
      return {
        ...nextBase,
        activeTurn: {
          turnId: event.turn_id,
          status: event.status,
          error: event.error,
        },
      };
  }
}
