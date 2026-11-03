import { z } from "zod";
import type {
  CmaEventItem,
  CmaSessionAction,
  CmaSessionState,
  CmaSessionStatus,
} from "@/schemas/cma-session";

const MAX_ACTIONS = 30;
const toolStartSchema = z.object({
  call_id: z.string(),
  title: z.string(),
  status: z.string(),
});
const toolUpdateSchema = z.object({
  call_id: z.string(),
  title: z.string().nullish(),
  status: z.string(),
});

export function cmaSessionStatus(status: string): CmaSessionStatus {
  switch (status.toLowerCase()) {
    case "working":
      return "working";
    case "ready":
      return "ready";
    case "waiting_approval":
      return "waiting";
    case "paused":
      return "paused";
    case "starting":
    case "provisioning":
      return "starting";
    case "failed":
      return "failed";
    case "stopped":
      return "stopped";
    case "deleted":
      return "deleted";
    default:
      return "unknown";
  }
}

function toolStatus(status: string): CmaSessionAction["status"] {
  switch (status) {
    case "in_progress":
    case "pending":
    case "running":
      return "running";
    case "completed":
      return "completed";
    case "failed":
    case "error":
    case "cancelled":
      return "failed";
    default:
      return "unknown";
  }
}

export function applyCmaEvent(
  state: CmaSessionState,
  event: CmaEventItem,
): CmaSessionState {
  const kind = event.kind;
  if (!kind) return state;
  const next = { ...state, updatedAt: event.appended_at };
  switch (kind.type) {
    case "turn_started":
      return { ...next, status: "working", actions: [], notice: undefined };
    case "turn_completed":
      return {
        ...next,
        status: "completed",
        notice: undefined,
        actions: settleRunning(state.actions),
      };
    case "turn_failed":
    case "work_failed":
      return {
        ...next,
        status: "failed",
        actions: settleRunning(state.actions),
        notice: "CMA reported a failure. Open the session in CMA for details.",
      };
    case "turn_cancelled":
    case "stopped":
      return {
        ...next,
        status: "stopped",
        notice: undefined,
        actions: settleRunning(state.actions),
      };
    case "deleted":
      return {
        ...next,
        status: "deleted",
        notice: undefined,
        actions: settleRunning(state.actions),
      };
    case "approval_requested":
    case "authentication_required":
      return {
        ...next,
        status: "waiting",
        notice:
          kind.type === "approval_requested"
            ? "CMA is waiting for approval."
            : "CMA is waiting for sign-in.",
      };
    case "approval_resolved":
      return { ...next, status: "working", notice: undefined };
    case "conversation_available":
      return {
        ...next,
        status: state.status === "starting" ? "ready" : state.status,
      };
    case "status_changed": {
      if (typeof kind.to !== "string") return state;
      const status = cmaSessionStatus(kind.to);
      return {
        ...next,
        status,
        actions: ["ready", "failed", "stopped", "deleted"].includes(status)
          ? settleRunning(state.actions)
          : state.actions,
        notice:
          state.status === "waiting" && status !== "waiting"
            ? undefined
            : state.notice,
      };
    }
    case "title_set":
      return typeof kind.title === "string"
        ? { ...next, title: kind.title.slice(0, 300) }
        : state;
    case "tool_started": {
      const tool = toolStartSchema.parse(kind);
      const action = {
        id: tool.call_id,
        title: tool.title.slice(0, 240),
        status: toolStatus(tool.status),
        at: event.appended_at,
      };
      return {
        ...next,
        actions: [
          ...state.actions.filter((entry) => entry.id !== action.id),
          action,
        ].slice(-MAX_ACTIONS),
      };
    }
    case "tool_updated": {
      const tool = toolUpdateSchema.parse(kind);
      const existing = state.actions.find((entry) => entry.id === tool.call_id);
      const action = {
        id: tool.call_id,
        title: tool.title?.slice(0, 240) || existing?.title || "CMA action",
        status: toolStatus(tool.status),
        at: existing?.at ?? event.appended_at,
      };
      return {
        ...next,
        actions: existing
          ? state.actions.map((entry) =>
              entry.id === action.id ? action : entry,
            )
          : [...state.actions, action].slice(-MAX_ACTIONS),
      };
    }
    case "history_gap":
      return { ...next, notice: "CMA reported a gap in its activity history." };
    default:
      return state;
  }
}

function settleRunning(
  actions: readonly CmaSessionAction[],
): readonly CmaSessionAction[] {
  return actions.map((action) =>
    action.status === "running" ? { ...action, status: "unknown" } : action,
  );
}

export interface CmaSseFrame {
  readonly event: string;
  readonly id?: string;
  readonly data: string;
}

/** A frame may contain message bodies; retain only one bounded frame at a time. */
export class CmaSseDecoder {
  private buffer = "";
  push(chunk: string): CmaSseFrame[] {
    this.buffer += chunk;
    const frames: CmaSseFrame[] = [];
    for (;;) {
      const boundary = /\r?\n\r?\n/.exec(this.buffer);
      if (!boundary) break;
      if (boundary.index > 4 * 1024 * 1024)
        throw new Error("CMA sent an activity update that is too large.");
      const raw = this.buffer.slice(0, boundary.index);
      this.buffer = this.buffer.slice(boundary.index + boundary[0].length);
      let event = "message";
      let id: string | undefined;
      const data: string[] = [];
      for (const line of raw.split(/\r?\n/)) {
        if (line.startsWith(":")) continue;
        const colon = line.indexOf(":");
        const field = colon < 0 ? line : line.slice(0, colon);
        const value = colon < 0 ? "" : line.slice(colon + 1).replace(/^ /, "");
        if (field === "event") event = value;
        else if (field === "id" && !value.includes("\0")) id = value;
        else if (field === "data") data.push(value);
      }
      if (data.length) frames.push({ event, id, data: data.join("\n") });
    }
    if (this.buffer.length > 4 * 1024 * 1024)
      throw new Error("CMA sent an activity update that is too large.");
    return frames;
  }
}
