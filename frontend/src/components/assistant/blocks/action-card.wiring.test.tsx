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

interface CapturedDialogProps {
  readonly open: boolean;
  readonly actionRequestId: string;
  readonly params: unknown;
  readonly onComplete: (completion: unknown) => void;
}

const { captureDialog, dialogCalls } = vi.hoisted(() => {
  const calls = new Map<string, CapturedDialogProps>();
  return {
    dialogCalls: calls,
    captureDialog:
      (variant: string) =>
      (props: CapturedDialogProps): null => {
        if (props.open) calls.set(variant, props);
        return null;
      },
  };
});

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
  AssistantKeyUpdateDialog: captureDialog("key_update"),
}));

vi.mock("@/components/assistant/assistant-key-delete-dialog", () => ({
  AssistantKeyDeleteDialog: captureDialog("key_delete"),
}));

vi.mock("@/components/assistant/assistant-key-scope-dialog", () => ({
  AssistantKeyScopeDialog: captureDialog("key_extend_scope"),
}));

vi.mock("@/components/assistant/assistant-key-bind-dialog", () => ({
  AssistantKeyBindDialog: captureDialog("key_bind_credential"),
}));

vi.mock("@/components/assistant/assistant-service-update-dialog", () => ({
  AssistantServiceUpdateDialog: captureDialog("service_update"),
}));

vi.mock("@/components/assistant/assistant-service-delete-dialog", () => ({
  AssistantServiceDeleteDialog: captureDialog("service_delete"),
}));

vi.mock("@/components/assistant/assistant-service-route-dialog", () => ({
  AssistantServiceRouteDialog: captureDialog("service_route"),
}));

vi.mock(
  "@/components/assistant/assistant-service-rotate-credential-dialog",
  () => ({
    AssistantServiceRotateCredentialDialog: captureDialog(
      "service_rotate_credential",
    ),
  }),
);

vi.mock("@/components/assistant/assistant-endpoint-update-dialog", () => ({
  AssistantEndpointUpdateDialog: captureDialog("endpoint_update"),
}));

vi.mock("@/components/assistant/assistant-endpoint-delete-dialog", () => ({
  AssistantEndpointDeleteDialog: captureDialog("endpoint_delete"),
}));

vi.mock("@/components/assistant/assistant-external-key-rotate-dialog", () => ({
  AssistantExternalKeyRotateDialog: captureDialog("external_key_rotate"),
}));

vi.mock("@/components/assistant/assistant-external-key-delete-dialog", () => ({
  AssistantExternalKeyDeleteDialog: captureDialog("external_key_delete"),
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
  readonly completion: unknown;
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
  const dialogCall = dialogCalls.get(options.variant);
  expect(dialogCall).toMatchObject({
    actionRequestId,
    params: options.dialogParams,
  });
  if (!dialogCall) throw new Error(`${options.variant} dialog did not mount.`);

  dialogCall.onComplete(options.completion);
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
      completion: "key-updated",
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
      completion: "key-deleted",
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
      completion: "key-scope-extended",
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
      completion: {
        keyId: "key-bound",
        userServiceId: "service-bind-1",
      },
      resource: {
        key: { keyId: "key-bound", userServiceId: "service-bind-1" },
      },
    });
  });

  it("wires service.update through its typed dialog and service report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "service.update",
      rawParams: {
        userServiceId: "service-update-1",
        name: "Build API",
        endpointUrl: "https://build.example.test/v2",
        authMethod: "header",
        authKeyName: "X-Build-Key",
      },
      variant: "service_update",
      normalizedParams: {
        variant: "service_update",
        user_service_id: "service-update-1",
        name: "Build API",
        endpoint_url: "https://build.example.test/v2",
        auth_method: "header",
        auth_key_name: "X-Build-Key",
      },
      cta: "Update service",
      dialogParams: {
        userServiceId: "service-update-1",
        name: "Build API",
        endpointUrl: "https://build.example.test/v2",
        authMethod: "header",
        authKeyName: "X-Build-Key",
      },
      completion: "service-updated",
      resource: { userService: { userServiceId: "service-updated" } },
    });
  });

  it("wires service.delete through its typed dialog and service report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "service.delete",
      rawParams: { userServiceId: "service-delete-1" },
      variant: "service_delete",
      normalizedParams: {
        variant: "service_delete",
        user_service_id: "service-delete-1",
      },
      cta: "Delete service",
      dialogParams: { userServiceId: "service-delete-1" },
      completion: "service-deleted",
      resource: { userService: { userServiceId: "service-deleted" } },
    });
  });

  it("wires service.route through its typed dialog and service report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "service.route",
      rawParams: {
        userServiceId: "service-route-1",
        viaNodeId: "node-route-1",
      },
      variant: "service_route",
      normalizedParams: {
        variant: "service_route",
        user_service_id: "service-route-1",
        via_node_id: "node-route-1",
      },
      cta: "Change routing",
      dialogParams: {
        userServiceId: "service-route-1",
        viaNodeId: "node-route-1",
      },
      completion: "service-routed",
      resource: { userService: { userServiceId: "service-routed" } },
    });
  });

  it("wires service.rotate_credential through its typed dialog and service report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "service.rotate_credential",
      rawParams: { userServiceId: "service-rotate-1" },
      variant: "service_rotate_credential",
      normalizedParams: {
        variant: "service_rotate_credential",
        user_service_id: "service-rotate-1",
      },
      cta: "Rotate credential",
      dialogParams: { userServiceId: "service-rotate-1" },
      completion: "service-credential-rotated",
      resource: {
        userService: { userServiceId: "service-credential-rotated" },
      },
    });
  });

  it("wires endpoint.update through its typed dialog and endpoint report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "endpoint.update",
      rawParams: {
        endpointId: "endpoint-update-1",
        label: "Build endpoint",
        endpointUrl: "https://build.example.test/v3",
        openapiSpecUrl: "https://build.example.test/openapi.json",
      },
      variant: "endpoint_update",
      normalizedParams: {
        variant: "endpoint_update",
        endpoint_id: "endpoint-update-1",
        label: "Build endpoint",
        endpoint_url: "https://build.example.test/v3",
        openapi_spec_url: "https://build.example.test/openapi.json",
      },
      cta: "Update endpoint",
      dialogParams: {
        endpointId: "endpoint-update-1",
        label: "Build endpoint",
        endpointUrl: "https://build.example.test/v3",
        openapiSpecUrl: "https://build.example.test/openapi.json",
      },
      completion: "endpoint-updated",
      resource: { endpoint: { endpointId: "endpoint-updated" } },
    });
  });

  it("wires endpoint.delete through its typed dialog and endpoint report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "endpoint.delete",
      rawParams: { endpointId: "endpoint-delete-1" },
      variant: "endpoint_delete",
      normalizedParams: {
        variant: "endpoint_delete",
        endpoint_id: "endpoint-delete-1",
      },
      cta: "Delete endpoint",
      dialogParams: { endpointId: "endpoint-delete-1" },
      completion: "endpoint-deleted",
      resource: { endpoint: { endpointId: "endpoint-deleted" } },
    });
  });

  it("wires external_key.rotate through its typed dialog and external-key report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "external_key.rotate",
      rawParams: { externalKeyId: "external-rotate-1" },
      variant: "external_key_rotate",
      normalizedParams: {
        variant: "external_key_rotate",
        external_key_id: "external-rotate-1",
      },
      cta: "Rotate external credential",
      dialogParams: { externalKeyId: "external-rotate-1" },
      completion: "external-key-rotated",
      resource: {
        externalKey: { externalKeyId: "external-key-rotated" },
      },
    });
  });

  it("wires external_key.delete through its typed dialog and external-key report", async () => {
    // Falsifiers exercised: deleting the registry row removes the dialog,
    // breaking toProps changes these props, and changing resource changes the report.
    await runJourney({
      action: "external_key.delete",
      rawParams: { externalKeyId: "external-delete-1" },
      variant: "external_key_delete",
      normalizedParams: {
        variant: "external_key_delete",
        external_key_id: "external-delete-1",
      },
      cta: "Delete external credential",
      dialogParams: { externalKeyId: "external-delete-1" },
      completion: "external-key-deleted",
      resource: {
        externalKey: { externalKeyId: "external-key-deleted" },
      },
    });
  });
});
