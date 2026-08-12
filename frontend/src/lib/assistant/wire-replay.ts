import { resolveAssistantAction } from "@/lib/assistant/action-registry";
import { ChatStreamParser } from "@/lib/assistant/chat-stream-parser";
import {
  type AuthorizationBlocker,
  parseToolResultBlocker,
  redactDisplayText,
  summarizeToolResult,
} from "@/lib/assistant/aevatar-transport";
import {
  applyTurnEvent,
  EMPTY_TURN_STATE,
  toTerminalBlock,
} from "@/lib/assistant/stream";
import {
  assistantActionRequestSchema,
  recoverUnsupportedAssistantActionRequest,
} from "@/schemas/assistant-actions";
import type {
  AssistantWireCaptureOutcome,
  AssistantWireLine,
  AssistantWireLogExchange,
} from "@/schemas/assistant-wire-log";
import type {
  ActionCardContentBlock,
  ApprovalCardContentBlock,
  ConnectCardContentBlock,
  ContentBlock,
  RunContentBlock,
  RunStepStatus,
  TurnEvent,
  TurnReducerState,
} from "@/types/assistant";
import type { ChatStreamFrame } from "@/lib/assistant/chat-stream-worker-protocol";

export type WireReplayProtocol = "actor";

export interface WireReplayContext {
  readonly protocol: WireReplayProtocol;
  readonly captureOutcome?: AssistantWireCaptureOutcome;
  readonly truncated: boolean;
}

export interface WireReplayProjection {
  readonly events: readonly TurnEvent[];
  readonly state: TurnReducerState;
  readonly sourceFramesByBlockId: Readonly<
    Record<string, readonly ChatStreamFrame[]>
  >;
  readonly partial: boolean;
}

type CursorlessTurnEvent = TurnEvent extends infer Event
  ? Event extends TurnEvent
    ? Omit<Event, "cursor">
    : never
  : never;

interface ReplayStep {
  readonly key: string;
  index: number;
  status: RunStepStatus;
  label: string;
  meta: string;
  approval_request_id: string | null;
}

interface ReplayTerminal {
  readonly kind: "finished" | "error" | "stopped";
  readonly status?: "completed" | "blocked";
  readonly error?: { readonly code: string; readonly message: string };
}

const TURN_ID_PATTERN = /^[A-Za-z0-9._:-]{1,256}$/;

function safeTurnId(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  return TURN_ID_PATTERN.test(normalized) ? normalized : null;
}

function safeErrorCode(value: unknown, fallback: string): string {
  return typeof value === "string" && /^[A-Za-z0-9._:-]{1,128}$/.test(value)
    ? value
    : fallback;
}

function safeErrorMessage(value: unknown, fallback: string): string {
  if (typeof value !== "string" || !value.trim()) return fallback;
  return redactDisplayText(value.trim()).slice(0, 1_024);
}

function objectValue(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
}

function unpackAny(value: unknown): Record<string, unknown> {
  const record = objectValue(value);
  const nested = record["value"];
  if (nested && typeof nested === "object" && !Array.isArray(nested)) {
    return nested as Record<string, unknown>;
  }
  const unpacked = { ...record };
  delete unpacked["@type"];
  return unpacked;
}

function stringValue(record: Record<string, unknown>, key: string): string {
  const value = record[key];
  return typeof value === "string" ? value : "";
}

function booleanValue(
  record: Record<string, unknown>,
  key: string,
): boolean | undefined {
  const value = record[key];
  return typeof value === "boolean" ? value : undefined;
}

function frameType(frame: ChatStreamFrame): string {
  const explicit = frame["type"];
  if (typeof explicit === "string") return explicit.toUpperCase();
  const bodyKeys: readonly [string, string][] = [
    ["runStarted", "RUN_STARTED"],
    ["runFinished", "RUN_FINISHED"],
    ["runStopped", "RUN_STOPPED"],
    ["runError", "RUN_ERROR"],
    ["stepStarted", "STEP_STARTED"],
    ["stepFinished", "STEP_FINISHED"],
    ["textMessageStart", "TEXT_MESSAGE_START"],
    ["textMessageContent", "TEXT_MESSAGE_CONTENT"],
    ["textMessageEnd", "TEXT_MESSAGE_END"],
    ["toolCallStart", "TOOL_CALL_START"],
    ["toolCallEnd", "TOOL_CALL_END"],
    ["toolApprovalRequest", "TOOL_APPROVAL_REQUEST"],
    ["authorizationRequired", "AUTHORIZATION_REQUIRED"],
    ["usage", "USAGE"],
    ["mediaContent", "MEDIA_CONTENT"],
    ["stateSnapshot", "STATE_SNAPSHOT"],
    ["custom", "CUSTOM"],
  ];
  return bodyKeys.find(([key]) => frame[key] !== undefined)?.[1] ?? "UNKNOWN";
}

function base64SizeBytes(value: string): number {
  return Math.floor((value.length * 3) / 4);
}

function humanizeSlug(slug: string): string {
  const bare = slug.replace(/^api-/, "");
  return (
    bare
      .split(/[-_]/)
      .filter(Boolean)
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(" ") || "NyxID service"
  );
}

class ReplayProjector {
  private readonly context: WireReplayContext;
  private readonly events: TurnEvent[] = [];
  private readonly sources = new Map<string, ChatStreamFrame[]>();
  private state: TurnReducerState = EMPTY_TURN_STATE;
  private cursor = 0;
  private id = 0;
  private turnId: string | null = null;
  private started = false;
  private terminal: ReplayTerminal | null = null;
  private terminalCount = 0;
  private protocolError: { code: string; message: string } | null = null;
  private currentMessageId: string | null = null;
  private currentBlockId: string | null = null;
  private currentText = "";
  private activityMessageId: string | null = null;
  private activityBlockCount = 0;
  private runBlockId: string | null = null;
  private readonly steps: ReplayStep[] = [];
  private readonly promptedApprovals = new Set<string>();
  private readonly promptedConnections = new Map<string, string>();
  private readonly promptedActions = new Map<string, string>();
  private readonly openCards = new Map<string, "approval" | "connect">();
  private waitingForApproval = false;
  /** Idle on purpose at an approval gate. */
  private get suspended(): boolean {
    return this.waitingForApproval;
  }

  constructor(context: WireReplayContext) {
    this.context = context;
  }

  project(frames: readonly ChatStreamFrame[]): WireReplayProjection {
    for (const frame of frames) {
      this.handle(frame);
      if (this.protocolError) break;
    }
    this.finish();
    return {
      events: this.events,
      state: this.state,
      sourceFramesByBlockId: Object.fromEntries(this.sources),
      partial:
        this.context.truncated ||
        this.context.captureOutcome !== "complete" ||
        (!this.terminal && !this.suspended),
    };
  }

  private nextId(prefix: string): string {
    this.id += 1;
    return `replay-${prefix}-${String(this.id)}`;
  }

  private emit(event: CursorlessTurnEvent): void {
    this.cursor += 1;
    const complete = { ...event, cursor: this.cursor } as TurnEvent;
    this.events.push(complete);
    this.state = applyTurnEvent(
      this.state,
      complete,
      new Date(0).toISOString(),
    );
  }

  private addSource(blockId: string, frame: ChatStreamFrame): void {
    this.sources.set(blockId, [...(this.sources.get(blockId) ?? []), frame]);
  }

  private failProtocol(message: string): void {
    this.protocolError ??= { code: "stream_protocol_error", message };
  }

  private handle(frame: ChatStreamFrame): void {
    const type = frameType(frame);
    const custom = objectValue(frame["custom"]);
    const keepalive =
      type === "CUSTOM" &&
      stringValue(custom, "name") === "aevatar.nyxid_chat.keepalive";

    if (
      this.terminal &&
      !keepalive &&
      type !== "RUN_FINISHED" &&
      type !== "RUN_ERROR" &&
      type !== "RUN_STOPPED"
    ) {
      this.failProtocol(
        "The assistant stream sent data after its terminal frame.",
      );
      return;
    }
    if (type !== "RUN_STARTED" && !keepalive && !this.started) {
      this.failProtocol(
        "The assistant stream sent data before identifying the turn.",
      );
      return;
    }

    switch (type) {
      case "RUN_STARTED":
        this.handleRunStarted(frame);
        return;
      case "TEXT_MESSAGE_START":
        this.startText(frame);
        return;
      case "TEXT_MESSAGE_CONTENT":
        this.appendText(frame);
        return;
      case "TEXT_MESSAGE_END":
        this.addSourceToCurrent(frame);
        this.closeText();
        return;
      case "TOOL_CALL_START": {
        const body = objectValue(frame["toolCallStart"]);
        this.startStep(
          stringValue(body, "toolCallId") || this.nextId("tool"),
          stringValue(body, "toolName") || "tool",
          frame,
        );
        return;
      }
      case "TOOL_CALL_END":
        this.finishToolCall(objectValue(frame["toolCallEnd"]), frame);
        return;
      case "TOOL_APPROVAL_REQUEST":
        this.addApproval(objectValue(frame["toolApprovalRequest"]), frame);
        return;
      case "MEDIA_CONTENT":
        this.addMedia(objectValue(frame["mediaContent"]), frame);
        return;
      case "STEP_STARTED": {
        const body = objectValue(frame["stepStarted"]);
        const name = stringValue(body, "stepName") || "step";
        this.startStep(name, name, frame);
        return;
      }
      case "STEP_FINISHED": {
        const body = objectValue(frame["stepFinished"]);
        const name = stringValue(body, "stepName") || "step";
        const succeeded = booleanValue(body, "success") !== false;
        this.finishStep(
          name,
          succeeded,
          succeeded ? "Completed" : "Step failed",
          frame,
        );
        return;
      }
      case "RUN_ERROR": {
        const body = objectValue(frame["runError"]);
        this.recordTerminal({
          kind: "error",
          error: {
            code: safeErrorCode(body["code"], "run_error"),
            message: safeErrorMessage(
              body["message"],
              "The assistant run failed.",
            ),
          },
        });
        return;
      }
      case "RUN_FINISHED": {
        const body = objectValue(frame["runFinished"]);
        const status = body["status"];
        if (
          status !== undefined &&
          status !== "completed" &&
          status !== "blocked"
        ) {
          this.failProtocol(
            "The assistant stream returned an unknown terminal status.",
          );
          return;
        }
        this.recordTerminal({
          kind: "finished",
          status: status === "blocked" ? "blocked" : "completed",
        });
        return;
      }
      case "RUN_STOPPED":
        this.recordTerminal({ kind: "stopped" });
        return;
      case "CUSTOM":
        this.handleCustom(custom, frame);
        return;
      default:
        return;
    }
  }

  private handleRunStarted(frame: ChatStreamFrame): void {
    const body = objectValue(frame["runStarted"]);
    const turnId = safeTurnId(
      frame["turnId"] ?? body["turnId"] ?? body["runId"],
    );
    if (this.started || !turnId) {
      this.failProtocol(
        this.started
          ? "The assistant stream started the same delivery more than once."
          : "The assistant stream did not provide a valid turn id.",
      );
      return;
    }
    if (this.turnId && this.turnId !== turnId) {
      this.failProtocol("The assistant replay changed the turn id.");
      return;
    }
    this.started = true;
    this.turnId ??= turnId;
    this.emit({ event: "turn.status", turn_id: turnId, status: "running" });
  }

  private startText(frame: ChatStreamFrame): void {
    const body = objectValue(frame["textMessageStart"]);
    const messageId =
      stringValue(body, "messageId") || this.nextId("assistant-message");
    this.currentMessageId = messageId;
    this.currentBlockId = `${messageId}-text`;
    this.currentText = "";
    this.addSource(this.currentBlockId, frame);
    this.emit({
      event: "message.started",
      message_id: messageId,
      role: "assistant",
    });
    this.emit({
      event: "block.started",
      message_id: messageId,
      block_id: this.currentBlockId,
      index: 0,
      block: { type: "text", block_id: this.currentBlockId, text: "" },
    });
  }

  private appendText(frame: ChatStreamFrame): void {
    const body = objectValue(frame["textMessageContent"]);
    const delta = stringValue(body, "delta");
    if (!delta || !this.currentBlockId) return;
    this.currentText += delta;
    this.addSource(this.currentBlockId, frame);
    this.emit({
      event: "block.delta",
      block_id: this.currentBlockId,
      text: delta,
    });
  }

  private addSourceToCurrent(frame: ChatStreamFrame): void {
    if (this.currentBlockId) this.addSource(this.currentBlockId, frame);
  }

  private closeText(): void {
    if (!this.currentMessageId || !this.currentBlockId) return;
    const messageId = this.currentMessageId;
    const blockId = this.currentBlockId;
    this.emit({
      event: "block.completed",
      block_id: blockId,
      block: { type: "text", block_id: blockId, text: this.currentText },
    });
    this.emit({ event: "message.completed", message_id: messageId });
    this.currentMessageId = null;
    this.currentBlockId = null;
    this.currentText = "";
  }

  private ensureActivityMessage(): string {
    if (this.activityMessageId) return this.activityMessageId;
    this.activityMessageId = this.nextId("assistant-activity");
    this.emit({
      event: "message.started",
      message_id: this.activityMessageId,
      role: "assistant",
    });
    return this.activityMessageId;
  }

  private appendActivityBlock(
    block: ContentBlock,
    frame: ChatStreamFrame,
  ): void {
    const messageId = this.ensureActivityMessage();
    this.addSource(block.block_id, frame);
    this.emit({
      event: "block.started",
      message_id: messageId,
      block_id: block.block_id,
      index: this.activityBlockCount,
      block,
    });
    this.activityBlockCount += 1;
  }

  private runSnapshot(): RunContentBlock {
    const stepsComplete = this.steps.filter(
      (step) => step.status === "done",
    ).length;
    return {
      type: "run",
      block_id: this.runBlockId ?? this.nextId("run-block"),
      title: "RUN",
      steps_total: this.steps.length,
      steps_complete: stepsComplete,
      state: this.waitingForApproval
        ? "awaiting_approval"
        : this.promptedConnections.size > 0
          ? "awaiting_connection"
          : "running",
      steps: this.steps.map((step) => ({
        index: step.index,
        status: step.status,
        label: step.label,
        meta: step.meta,
        service_slug: null,
        artifact_id: null,
        approval_request_id: step.approval_request_id,
      })),
    };
  }

  private patchRun(frame: ChatStreamFrame): void {
    const snapshot = this.runSnapshot();
    if (!this.runBlockId) {
      this.runBlockId = snapshot.block_id;
      this.appendActivityBlock(snapshot, frame);
      return;
    }
    this.addSource(this.runBlockId, frame);
    this.emit({
      event: "block.updated",
      block_id: this.runBlockId,
      patch: {
        steps_total: snapshot.steps_total,
        steps_complete: snapshot.steps_complete,
        state: snapshot.state,
        steps: snapshot.steps,
      },
    });
  }

  private startStep(key: string, label: string, frame: ChatStreamFrame): void {
    if (this.steps.some((step) => step.key === key)) return;
    this.steps.push({
      key,
      index: this.steps.length + 1,
      status: "active",
      label: redactDisplayText(label),
      meta: "Running",
      approval_request_id: null,
    });
    this.patchRun(frame);
  }

  private finishStep(
    key: string,
    succeeded: boolean,
    meta: string,
    frame: ChatStreamFrame,
  ): void {
    let step = this.steps.find((candidate) => candidate.key === key);
    if (!step) {
      this.startStep(key, key || "tool", frame);
      step = this.steps.find((candidate) => candidate.key === key);
    }
    if (!step) return;
    step.status = succeeded ? "done" : "failed";
    step.meta = meta;
    this.patchRun(frame);
  }

  private finishToolCall(
    body: Record<string, unknown>,
    frame: ChatStreamFrame,
  ): void {
    const namedKey = stringValue(body, "toolCallId");
    const key = namedKey || this.steps.at(-1)?.key || this.nextId("tool");
    if (this.steps.length === 0) {
      this.startStep(key, stringValue(body, "toolName") || "tool", frame);
    }
    const status = stringValue(body, "status").toUpperCase();
    // AG-UI `toolCallEnd` may be only `{toolCallId, result}`, so a blocked
    // capability can be visible solely inside the result. Keep replay aligned
    // with the live transport for that typed shape.
    const blocker = parseToolResultBlocker(body["result"]);
    if (blocker) this.addConnectionBlocker(blocker, frame);
    const succeeded =
      !blocker &&
      booleanValue(body, "success") !== false &&
      !/(ERROR|DENIED)/.test(status);
    this.finishStep(
      key,
      succeeded,
      blocker
        ? blocker.safeMessage
        : summarizeToolResult(
            succeeded ? body["result"] : (body["result"] ?? body["error"]),
          ),
      frame,
    );
  }

  private addApproval(
    body: Record<string, unknown>,
    frame: ChatStreamFrame,
  ): void {
    const requestId =
      stringValue(body, "requestId") ||
      stringValue(body, "approvalRequestId") ||
      stringValue(body, "commandId");
    if (!requestId || this.promptedApprovals.has(requestId)) return;
    this.promptedApprovals.add(requestId);
    this.waitingForApproval = true;
    const toolName = stringValue(body, "toolName");
    const block: ApprovalCardContentBlock = {
      type: "approval_card",
      block_id: this.nextId("approval-card"),
      approval_request_id: requestId,
      body: redactDisplayText(
        stringValue(body, "message") ||
          stringValue(body, "body") ||
          (toolName
            ? `The assistant wants to run ${redactDisplayText(toolName)}.`
            : "The assistant is requesting your approval to continue."),
      ),
      service_slug:
        stringValue(body, "serviceSlug") || stringValue(body, "service_slug"),
      agent_key_prefix: stringValue(body, "agentKeyPrefix") || "aevatar",
      approval_mode:
        stringValue(body, "approvalMode") === "grant" ? "grant" : "per_request",
      grant_duration_sec:
        typeof body["grantDurationSec"] === "number"
          ? body["grantDurationSec"]
          : null,
      expires_at:
        stringValue(body, "expiresAt") || stringValue(body, "expires_at"),
      decision: null,
      decision_channel: null,
      decision_submission: null,
    };
    this.appendActivityBlock(block, frame);
    this.openCards.set(block.block_id, "approval");
    const stepKey =
      stringValue(body, "toolCallId") || stringValue(body, "stepId");
    const step = stepKey
      ? this.steps.find((candidate) => candidate.key === stepKey)
      : [...this.steps]
          .reverse()
          .find((candidate) => candidate.status === "active");
    if (step?.status === "active") {
      step.status = "waiting";
      step.approval_request_id = requestId;
      this.patchRun(frame);
    }
    if (this.runBlockId) this.patchRun(frame);
    if (this.turnId) {
      this.emit({
        event: "turn.status",
        turn_id: this.turnId,
        status: "waiting",
      });
    }
  }

  /**
   * Render a blocker recovered from a tool result through the same card path
   * as a typed `nyxid.authorization.required` frame, by handing it back in
   * that frame's camelCase shape.
   */
  private addConnectionBlocker(
    blocker: AuthorizationBlocker,
    frame: ChatStreamFrame,
  ): void {
    this.addConnection(
      {
        reasonCode: blocker.reasonCode,
        serviceSlug: blocker.serviceSlug,
        serviceLabel: blocker.serviceLabel,
        safeMessage: blocker.safeMessage,
      },
      frame,
    );
  }

  private addConnection(payload: unknown, frame: ChatStreamFrame): void {
    const body = unpackAny(payload);
    const reasonCode = body["reasonCode"];
    if (
      reasonCode !== "NYXID_SERVICE_NOT_CONNECTED" &&
      reasonCode !== "NYXID_UNAUTHORIZED"
    )
      return;
    const rawSlug = body["serviceSlug"];
    if (typeof rawSlug !== "string") return;
    const slug = rawSlug.trim().toLowerCase();
    if (!/^[a-z0-9][a-z0-9-]{0,127}$/.test(slug)) return;
    const label =
      typeof body["serviceLabel"] === "string" && body["serviceLabel"].trim()
        ? safeErrorMessage(body["serviceLabel"], humanizeSlug(slug)).slice(
            0,
            128,
          )
        : humanizeSlug(slug);
    const message =
      typeof body["safeMessage"] === "string" && body["safeMessage"].trim()
        ? safeErrorMessage(
            body["safeMessage"],
            `Connect or reauthorize ${slug} to continue.`,
          )
        : `Connect or reauthorize ${slug} to continue.`;
    const existingId = this.promptedConnections.get(slug);
    if (existingId) {
      this.addSource(existingId, frame);
      this.emit({
        event: "block.updated",
        block_id: existingId,
        patch: {
          service_name: label,
          reason_code: reasonCode,
          steps: [
            {
              title:
                reasonCode === "NYXID_UNAUTHORIZED"
                  ? `Reconnect ${label}`
                  : `Connect ${label}`,
              body: message,
              done: false,
            },
          ],
        },
      });
      return;
    }
    const block: ConnectCardContentBlock = {
      type: "connect_card",
      block_id: this.nextId("connect-card"),
      catalog_slug: slug || "custom",
      service_name: label,
      icon_url: "",
      subtitle: "Required by this request",
      auth_kind: "api_key",
      requested_scopes: [],
      key_id: null,
      granted_scopes: null,
      device_user_code: null,
      device_verification_url: null,
      state: "needs_connection",
      error_message: null,
      steps: [
        {
          title:
            reasonCode === "NYXID_UNAUTHORIZED"
              ? `Reconnect ${label}`
              : `Connect ${label}`,
          body: message,
          done: false,
        },
      ],
      footer: "Brokered by NyxID · configure in AI Services, then ask again",
      reason_code: reasonCode,
    };
    this.appendActivityBlock(block, frame);
    this.promptedConnections.set(slug, block.block_id);
    this.openCards.set(block.block_id, "connect");
    if (this.runBlockId) this.patchRun(frame);
  }

  private addAction(payload: unknown, frame: ChatStreamFrame): void {
    const body = unpackAny(payload);
    const parsed = assistantActionRequestSchema.safeParse(body);
    const request = parsed.success
      ? parsed.data
      : recoverUnsupportedAssistantActionRequest(body);
    if (!request || this.promptedActions.has(request.actionRequestId)) return;
    const resolved = resolveAssistantAction(request);
    const block: ActionCardContentBlock = {
      type: "action_card",
      block_id: this.nextId("action-card"),
      action: request.action,
      action_request_id: request.actionRequestId,
      origin_turn_id: request.originTurnId,
      actor_id: request.actorId || undefined,
      task_id: request.taskId,
      step_id: request.stepId,
      params: resolved.params,
      status: resolved.supported ? "pending" : "unsupported",
      outcome_note: "",
    };
    this.appendActivityBlock(block, frame);
    this.promptedActions.set(request.actionRequestId, block.block_id);
  }

  private addMedia(
    body: Record<string, unknown>,
    frame: ChatStreamFrame,
  ): void {
    const name = stringValue(body, "name").trim() || "attachment";
    const mime = stringValue(body, "mediaType") || "application/octet-stream";
    const data = stringValue(body, "dataBase64");
    const url = stringValue(body, "url");
    if (data && data.length <= 8 * 1024 * 1024) {
      this.appendActivityBlock(
        {
          type: "artifact",
          block_id: this.nextId("artifact"),
          artifact_id: this.nextId("media"),
          name,
          mime,
          size_bytes: base64SizeBytes(data),
          preview: null,
          download_url: `data:${mime};base64,${data}`,
        },
        frame,
      );
      return;
    }
    if (url) {
      this.appendActivityBlock(
        {
          type: "artifact",
          block_id: this.nextId("artifact"),
          artifact_id: this.nextId("media"),
          name,
          mime,
          size_bytes: 0,
          preview: null,
          download_url: url,
        },
        frame,
      );
      return;
    }
    this.emitStaticText(
      `The assistant produced an attachment (${name}) that is too large to display here.`,
      frame,
    );
  }

  private handleCustom(
    custom: Record<string, unknown>,
    frame: ChatStreamFrame,
  ): void {
    const name = stringValue(custom, "name");
    const payload = custom["payload"];
    switch (name) {
      case "aevatar.nyxid_chat.keepalive":
      case "aevatar.llm.reasoning":
      case "aevatar.run.context":
      case "demo.conversation.context":
        return;
      case "nyxid.authorization.required":
        this.addConnection(payload, frame);
        return;
      case "nyxid.action.request":
        this.addAction(payload, frame);
        return;
      case "aevatar.tool_approval.pending":
        this.addApproval(unpackAny(payload), frame);
        return;
      case "aevatar.step.completed": {
        const body = unpackAny(payload);
        // proto3 elides a `false` bool, so an absent `success` IS a failure.
        // Must match the live transport or replay parity drifts on failures.
        const succeeded = booleanValue(body, "success") === true;
        this.finishStep(
          stringValue(body, "stepId"),
          succeeded,
          succeeded ? "Completed" : "Step failed",
          frame,
        );
        return;
      }
      default:
        return;
    }
  }

  private emitStaticText(text: string, frame: ChatStreamFrame): void {
    const messageId = this.nextId("assistant-message");
    const blockId = `${messageId}-text`;
    this.addSource(blockId, frame);
    this.emit({
      event: "message.started",
      message_id: messageId,
      role: "assistant",
    });
    this.emit({
      event: "block.started",
      message_id: messageId,
      block_id: blockId,
      index: 0,
      block: { type: "text", block_id: blockId, text },
    });
    this.emit({
      event: "block.completed",
      block_id: blockId,
      block: { type: "text", block_id: blockId, text },
    });
    this.emit({ event: "message.completed", message_id: messageId });
  }

  private recordTerminal(terminal: ReplayTerminal): void {
    this.terminalCount += 1;
    if (this.terminalCount !== 1) {
      this.failProtocol(
        "The assistant stream sent more than one terminal frame.",
      );
      return;
    }
    this.terminal = terminal;
  }

  private finalizeActivity(
    outcome: "done" | "waiting" | "blocked" | "failed" | "cancelled",
  ): void {
    if (this.runBlockId) {
      for (const step of this.steps) {
        if (step.status === "active") {
          step.status =
            outcome === "done"
              ? "done"
              : outcome === "failed" || outcome === "blocked"
                ? "failed"
                : outcome === "cancelled"
                  ? "skipped"
                  : step.status;
          if (step.status === "done" && step.meta === "Running")
            step.meta = "Completed";
        } else if (
          step.status === "waiting" &&
          (outcome === "blocked" ||
            outcome === "failed" ||
            outcome === "cancelled")
        ) {
          step.status = "skipped";
        }
      }
      const snapshot = this.runSnapshot();
      const state =
        outcome === "done"
          ? this.steps.some((step) => step.status === "failed")
            ? "failed"
            : "completed"
          : outcome === "waiting"
            ? "awaiting_approval"
            : outcome === "blocked"
              ? "awaiting_connection"
              : outcome === "failed"
                ? "failed"
                : "cancelled";
      this.emit({
        event: "block.completed",
        block_id: this.runBlockId,
        block: { ...snapshot, block_id: this.runBlockId, state },
      });
    }
    for (const [blockId, kind] of this.openCards) {
      const block = this.state.messages
        .flatMap((message) => message.blocks)
        .find((candidate) => candidate.block_id === blockId);
      if (!block) continue;
      const terminal =
        outcome === "cancelled" ||
        (kind === "approval" && (outcome === "blocked" || outcome === "failed"))
          ? toTerminalBlock(block)
          : block;
      this.emit({
        event: "block.completed",
        block_id: blockId,
        block: terminal,
      });
    }
    if (this.activityMessageId) {
      this.emit({
        event: "message.completed",
        message_id: this.activityMessageId,
      });
    }
  }

  private completeTurn(
    status: "blocked" | "completed" | "failed" | "cancelled",
    error: { code: string; message: string } | null,
  ): void {
    this.emit({ event: "turn.completed", turn_id: this.turnId, status, error });
  }

  private finish(): void {
    this.closeText();
    if (this.protocolError) {
      this.finalizeActivity("failed");
      this.completeTurn("failed", this.protocolError);
      return;
    }
    if (this.terminal) {
      if (this.terminal.kind === "error") {
        this.finalizeActivity("failed");
        this.completeTurn("failed", this.terminal.error ?? null);
      } else if (this.terminal.kind === "stopped") {
        this.finalizeActivity("cancelled");
        if (this.turnId) {
          this.emit({
            event: "turn.status",
            turn_id: this.turnId,
            status: "cancelled",
          });
        }
        this.completeTurn("cancelled", null);
      } else if (this.terminal.status === "blocked") {
        this.finalizeActivity("blocked");
        this.completeTurn("blocked", null);
      } else {
        this.finalizeActivity(this.suspended ? "waiting" : "done");
        this.completeTurn("completed", null);
      }
      return;
    }
    if (this.suspended && this.context.captureOutcome === "complete") {
      this.finalizeActivity("waiting");
      this.completeTurn("completed", null);
      return;
    }
    if (
      this.context.captureOutcome === "cancelled" ||
      this.context.captureOutcome === "protocol_cancel"
    ) {
      this.finalizeActivity("cancelled");
      this.completeTurn("cancelled", null);
      return;
    }
    this.finalizeActivity("failed");
    const code =
      this.context.captureOutcome === "worker_error"
        ? "worker_error"
        : this.context.captureOutcome === "network_error"
          ? "network_error"
          : "stream_closed";
    this.completeTurn("failed", {
      code,
      message:
        code === "stream_closed"
          ? "The stream ended before the assistant finished. The reply may be incomplete; it will appear in full once the conversation reloads."
          : "The captured assistant stream ended before a terminal frame.",
    });
  }
}

/** Replay-only projector. Live delivery remains owned by AevatarAssistantTransport. */
export function projectReplayFrames(
  frames: readonly ChatStreamFrame[],
  context: WireReplayContext,
): WireReplayProjection {
  return new ReplayProjector(context).project(frames);
}

export function parseCapturedWireLines(
  lines: readonly AssistantWireLine[],
): ChatStreamFrame[] {
  const parser = new ChatStreamParser();
  const bytes = new TextEncoder().encode(
    lines.map((line) => `${line.text}${line.ending}`).join(""),
  );
  return [...parser.push(bytes), ...parser.finish()];
}

export function replayWireExchange(
  exchange: AssistantWireLogExchange,
): WireReplayProjection | null {
  const capture = exchange.capture;
  if (!capture || capture.state === "evicted" || !capture.sse) return null;
  return projectReplayFrames(parseCapturedWireLines(capture.sse.lines), {
    protocol: "actor",
    captureOutcome: capture.state === "settled" ? capture.outcome : undefined,
    truncated: capture.sse.truncated,
  });
}
