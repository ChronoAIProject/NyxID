import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { resolveAssistantAction } from "@/lib/assistant/action-registry";
import {
  ACTION_SCHEMA_VERSION,
  assistantActionRequestSchema,
  type ActionCardParams,
  type ActionResource,
} from "@/schemas/assistant-actions";
import type { ActionCardContentBlock } from "@/types/assistant";
import { ActionCard } from "./action-card";

interface MockDialogProps<P, C = string> {
  readonly open: boolean;
  readonly actionRequestId: string;
  readonly params: P;
  readonly onComplete: (completion: C) => void;
}

const { dialogCalls } = vi.hoisted(() => ({
  dialogCalls: new Map<string, unknown>(),
}));

vi.mock("@/components/dashboard/add-key-dialog", () => ({
  AddKeyDialog: () => null,
}));

vi.mock("@/components/service-icon", () => ({
  ServiceIcon: () => null,
}));

vi.mock("@/hooks/use-chat-presence", () => ({
  useChatPresence: () => ({ visible: true, lastActivityAt: 0 }),
}));

vi.mock("@/hooks/use-keys", () => ({
  KEY_AUTH_FAILED: "failed",
  useKeyAuthorizationWatch: () => ({
    status: "idle",
    authorized: false,
    timedOut: false,
    errorMessage: null,
  }),
}));

vi.mock("@/components/assistant/assistant-key-update-dialog", () => ({
  AssistantKeyUpdateDialog: (
    props: MockDialogProps<{
      readonly keyId: string;
      readonly name?: string;
      readonly platform?: string;
      readonly description?: string;
    }>,
  ) => {
    if (!props.open) return null;
    dialogCalls.set("key_update", props);
    return (
      <button type="button" onClick={() => props.onComplete("key-updated")}>
        Finish key_update
      </button>
    );
  },
}));

vi.mock("@/components/assistant/assistant-key-delete-dialog", () => ({
  AssistantKeyDeleteDialog: (
    props: MockDialogProps<{ readonly keyId: string }>,
  ) => {
    if (!props.open) return null;
    dialogCalls.set("key_delete", props);
    return (
      <button type="button" onClick={() => props.onComplete("key-deleted")}>
        Finish key_delete
      </button>
    );
  },
}));

vi.mock("@/components/assistant/assistant-key-scope-dialog", () => ({
  AssistantKeyScopeDialog: (
    props: MockDialogProps<{
      readonly keyId: string;
      readonly addServiceIds: readonly string[];
    }>,
  ) => {
    if (!props.open) return null;
    dialogCalls.set("key_extend_scope", props);
    return (
      <button
        type="button"
        onClick={() => props.onComplete("key-scope-extended")}
      >
        Finish key_extend_scope
      </button>
    );
  },
}));

vi.mock("@/components/assistant/assistant-key-bind-dialog", () => ({
  AssistantKeyBindDialog: (
    props: MockDialogProps<
      {
        readonly keyId: string;
        readonly userServiceId: string;
        readonly externalKeyId: string;
      },
      { readonly keyId: string; readonly userServiceId: string }
    >,
  ) => {
    if (!props.open) return null;
    dialogCalls.set("key_bind_credential", props);
    return (
      <button
        type="button"
        onClick={() =>
          props.onComplete({
            keyId: "key-bound",
            userServiceId: props.params.userServiceId,
          })
        }
      >
        Finish key_bind_credential
      </button>
    );
  },
}));

beforeEach(() => {
  dialogCalls.clear();
});

interface JourneyOptions {
  readonly action: string;
  readonly rawParams: Readonly<Record<string, unknown>>;
  readonly variant: ActionCardParams["variant"];
  readonly normalizedParams: ActionCardParams;
  readonly cta: string;
  readonly dialogParams: Readonly<Record<string, unknown>>;
  readonly resource: ActionResource;
}

async function runJourney(options: JourneyOptions): Promise<void> {
  const actionRequestId = `act-${options.variant}`;
  const request = assistantActionRequestSchema.parse({
    schemaVersion: ACTION_SCHEMA_VERSION,
    actorId: "conversation-1",
    originTurnId: "turn-origin-1",
    taskId: "task-1",
    stepId: "step-1",
    actionRequestId,
    action: options.action,
    params: options.rawParams,
  });
  const resolved = resolveAssistantAction(request);
  expect(resolved).toMatchObject({
    supported: true,
    journey: options.variant,
    params: options.normalizedParams,
  });

  const block: ActionCardContentBlock = {
    type: "action_card",
    block_id: `block-${options.variant}`,
    action: options.action,
    action_request_id: actionRequestId,
    origin_turn_id: request.originTurnId,
    task_id: request.taskId,
    step_id: request.stepId,
    params: resolved.params,
    status: "pending",
    outcome_note: "",
  };
  const onResolve = vi.fn();
  render(
    <ActionCard
      block={block}
      onProgress={vi.fn()}
      onBlock={vi.fn()}
      onResolve={onResolve}
    />,
  );

  fireEvent.click(screen.getByRole("button", { name: options.cta }));
  expect(dialogCalls.get(options.variant)).toMatchObject({
    actionRequestId,
    params: options.dialogParams,
  });

  fireEvent.click(
    screen.getByRole("button", { name: `Finish ${options.variant}` }),
  );
  await waitFor(() => {
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId,
      originTurnId: request.originTurnId,
      disposition: "completed",
      resource: options.resource,
    });
  });
}

describe("Wave-2 action card wiring", () => {
  it("wires key.update through its typed dialog and key report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "key.update",
      rawParams: {
        keyId: "key-update-1",
        name: "Build agent",
        platform: "codex",
        description: "Build automation",
      },
      variant: "key_update",
      normalizedParams: {
        variant: "key_update",
        key_id: "key-update-1",
        name: "Build agent",
        platform: "codex",
        description: "Build automation",
      },
      cta: "Update key",
      dialogParams: {
        keyId: "key-update-1",
        name: "Build agent",
        platform: "codex",
        description: "Build automation",
      },
      resource: { key: { keyId: "key-updated" } },
    });
  });

  it("wires key.delete through its typed dialog and key report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "key.delete",
      rawParams: { keyId: "key-delete-1" },
      variant: "key_delete",
      normalizedParams: { variant: "key_delete", key_id: "key-delete-1" },
      cta: "Delete key",
      dialogParams: { keyId: "key-delete-1" },
      resource: { key: { keyId: "key-deleted" } },
    });
  });

  it("wires key.extend_scope through its typed dialog and key report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "key.extend_scope",
      rawParams: {
        keyId: "key-scope-1",
        addServiceIds: ["service-a", "service-b"],
      },
      variant: "key_extend_scope",
      normalizedParams: {
        variant: "key_extend_scope",
        key_id: "key-scope-1",
        add_service_ids: ["service-a", "service-b"],
      },
      cta: "Extend scope",
      dialogParams: {
        keyId: "key-scope-1",
        addServiceIds: ["service-a", "service-b"],
      },
      resource: { key: { keyId: "key-scope-extended" } },
    });
  });

  it("wires key.bind_credential through its typed dialog and compound key report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "key.bind_credential",
      rawParams: {
        keyId: "key-bind-1",
        userServiceId: "service-bind-1",
        externalKeyId: "external-bind-1",
      },
      variant: "key_bind_credential",
      normalizedParams: {
        variant: "key_bind_credential",
        key_id: "key-bind-1",
        user_service_id: "service-bind-1",
        external_key_id: "external-bind-1",
      },
      cta: "Bind credential",
      dialogParams: {
        keyId: "key-bind-1",
        userServiceId: "service-bind-1",
        externalKeyId: "external-bind-1",
      },
      resource: {
        key: { keyId: "key-bound", userServiceId: "service-bind-1" },
      },
    });
  });
});
