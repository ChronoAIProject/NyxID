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
  readonly action?: unknown;
  readonly params?: unknown;
  readonly onComplete: (completion: unknown) => void;
}

const { captureActionDialog, captureDialog, dialogCalls } = vi.hoisted(() => {
  const calls = new Map<string, CapturedDialogProps>();
  const capture = (variant: string, props: CapturedDialogProps): null => {
    if (props.open) calls.set(variant, props);
    return null;
  };
  return {
    dialogCalls: calls,
    captureDialog:
      (variant: string) =>
      (props: CapturedDialogProps): null => {
        return capture(variant, props);
      },
    captureActionDialog:
      (variants: Readonly<Record<string, string>>) =>
      (props: CapturedDialogProps): null => {
        const variant = variants[String(props.action)];
        if (!variant)
          throw new Error(`Unexpected dialog action: ${String(props.action)}`);
        return capture(variant, props);
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

vi.mock(
  "@/components/assistant/assistant-service-access-review-dialog",
  () => ({
    AssistantServiceAccessReviewDialog: captureDialog("service_access_review"),
  }),
);

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

vi.mock("@/components/assistant/assistant-connection-revoke-dialog", () => ({
  AssistantConnectionRevokeDialog: captureDialog("connection_revoke"),
}));

vi.mock("@/components/assistant/assistant-provider-disconnect-dialog", () => ({
  AssistantProviderDisconnectDialog: captureDialog("provider_disconnect"),
}));

vi.mock(
  "@/components/assistant/assistant-provider-set-app-credentials-dialog",
  () => ({
    AssistantProviderSetAppCredentialsDialog: captureDialog(
      "provider_set_app_credentials",
    ),
  }),
);

vi.mock("@/components/assistant/assistant-node-register-token-dialog", () => ({
  AssistantNodeRegisterTokenDialog: captureDialog("node_register_token"),
}));

vi.mock("@/components/assistant/assistant-node-rotate-token-dialog", () => ({
  AssistantNodeRotateTokenDialog: captureDialog("node_rotate_token"),
}));

vi.mock("@/components/assistant/assistant-node-delete-dialog", () => ({
  AssistantNodeDeleteDialog: captureDialog("node_delete"),
}));

vi.mock("@/components/assistant/assistant-node-transfer-dialog", () => ({
  AssistantNodeTransferDialog: captureDialog("node_transfer"),
}));

vi.mock(
  "@/components/assistant/assistant-node-inject-credential-dialog",
  () => ({
    AssistantNodeInjectCredentialDialog: captureDialog(
      "node_inject_credential",
    ),
  }),
);

vi.mock(
  "@/components/assistant/assistant-pending-credential-push-dialog",
  () => ({
    AssistantPendingCredentialPushDialog: captureDialog(
      "pending_credential_push",
    ),
  }),
);

vi.mock(
  "@/components/assistant/assistant-pending-credential-cancel-dialog",
  () => ({
    AssistantPendingCredentialCancelDialog: captureDialog(
      "pending_credential_cancel",
    ),
  }),
);

vi.mock("@/components/assistant/assistant-device-onboard-dialog", () => ({
  AssistantDeviceOnboardDialog: captureDialog("device_onboard"),
}));

vi.mock("@/components/assistant/assistant-org-action-dialog", () => ({
  AssistantOrgActionDialog: captureActionDialog({
    create: "org_create",
    update: "org_update",
    delete: "org_delete",
    member_add: "org_member_add",
    member_remove: "org_member_remove",
    member_update_role: "org_member_update_role",
    invite: "org_invite",
    set_primary: "org_set_primary",
  }),
}));

vi.mock(
  "@/components/assistant/assistant-account-profile-update-dialog",
  () => ({
    AssistantAccountProfileUpdateDialog: captureDialog(
      "account_profile_update",
    ),
  }),
);

vi.mock(
  "@/components/assistant/assistant-account-revoke-consent-dialog",
  () => ({
    AssistantAccountRevokeConsentDialog: captureDialog(
      "account_revoke_consent",
    ),
  }),
);

vi.mock("@/components/assistant/assistant-account-delete-dialog", () => ({
  AssistantAccountDeleteDialog: captureDialog("account_delete"),
}));

vi.mock("@/components/assistant/assistant-account-mfa-setup-dialog", () => ({
  AssistantAccountMfaSetupDialog: captureDialog("account_mfa_setup"),
}));

vi.mock("@/components/assistant/assistant-approval-configure-dialog", () => ({
  AssistantApprovalConfigureDialog: captureDialog("approval_configure"),
}));

vi.mock("@/components/assistant/assistant-approval-enable-dialog", () => ({
  AssistantApprovalEnableDialog: captureDialog("approval_enable"),
}));

vi.mock("@/components/assistant/assistant-approval-disable-dialog", () => ({
  AssistantApprovalDisableDialog: captureDialog("approval_disable"),
}));

vi.mock(
  "@/components/assistant/assistant-approval-revoke-grant-dialog",
  () => ({
    AssistantApprovalRevokeGrantDialog: captureDialog("approval_revoke_grant"),
  }),
);

vi.mock("@/components/assistant/assistant-notifications-action-dialog", () => ({
  AssistantNotificationsActionDialog: captureActionDialog({
    update: "notifications_update",
    telegram_link: "notifications_telegram_link",
    telegram_disconnect: "notifications_telegram_disconnect",
  }),
}));

vi.mock(
  "@/components/assistant/assistant-service-account-action-dialog",
  () => ({
    AssistantServiceAccountActionDialog: captureActionDialog({
      create: "service_account_create",
      update: "service_account_update",
      delete: "service_account_delete",
      rotate_secret: "service_account_rotate_secret",
      revoke_tokens: "service_account_revoke_tokens",
    }),
  }),
);

vi.mock("@/components/assistant/assistant-developer-app-action-dialog", () => ({
  AssistantDeveloperAppActionDialog: captureActionDialog({
    create: "developer_app_create",
    update: "developer_app_update",
    delete: "developer_app_delete",
    rotate_secret: "developer_app_rotate_secret",
  }),
}));

vi.mock(
  "@/components/assistant/assistant-org-integration-action-dialog",
  () => ({
    AssistantOrgIntegrationActionDialog: captureActionDialog({
      "external_key.add_gcp_service_account":
        "external_key_add_gcp_service_account",
      "openclaw.connect": "openclaw_connect",
    }),
  }),
);

beforeEach(() => {
  dialogCalls.clear();
});

interface JourneyOptions {
  readonly action: string;
  readonly rawParams: Readonly<Record<string, unknown>>;
  readonly variant: ActionCardParams["variant"];
  readonly normalizedParams: ActionCardParams;
  readonly cta: string;
  readonly dialogParams?: Readonly<Record<string, unknown>>;
  readonly dialogAction?: string;
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
  const expectedDialogProps: Record<string, unknown> = { actionRequestId };
  if (options.dialogParams !== undefined) {
    expectedDialogProps["params"] = options.dialogParams;
  }
  if (options.dialogAction !== undefined) {
    expectedDialogProps["action"] = options.dialogAction;
  }
  expect(dialogCall).toMatchObject(expectedDialogProps);
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
  it("wires service.access_review through its typed dialog and exact report", async () => {
    await runJourney({
      action: "service.access_review",
      rawParams: {
        serviceAccessReview: {
          userServiceId: "service-review-1",
          serviceSlug: "github",
          resourceUri: "https://nyxid.example/api/v1/proxy/s/github",
        },
      },
      variant: "service_access_review",
      normalizedParams: {
        variant: "service_access_review",
        user_service_id: "service-review-1",
        service_slug: "github",
        resource_uri: "https://nyxid.example/api/v1/proxy/s/github",
      },
      cta: "Review access",
      dialogParams: {
        userServiceId: "service-review-1",
        serviceSlug: "github",
        resourceUri: "https://nyxid.example/api/v1/proxy/s/github",
      },
      completion: "service-review-1",
      resource: { userService: { userServiceId: "service-review-1" } },
    });
  });

  it("declines service.access_review without opening the effect dialog", async () => {
    const request = assistantActionRequestSchema.parse({
      schemaVersion: ACTION_SCHEMA_VERSION,
      actorId: "nyxid-chat-card-1",
      originTurnId: "turn-review-1",
      taskId: "task-review-1",
      stepId: "step-review-1",
      actionRequestId: "act-review-decline",
      action: "service.access_review",
      params: {
        serviceAccessReview: {
          userServiceId: "service-review-1",
          serviceSlug: "github",
          resourceUri: "https://nyxid.example/api/v1/proxy/s/github",
        },
      },
    });
    const resolved = resolveAssistantAction(request);
    const onResolve = vi.fn();
    render(
      <ActionCard
        block={{
          type: "action_card",
          block_id: "block-review-decline",
          action: request.action,
          action_request_id: request.actionRequestId,
          origin_turn_id: request.originTurnId,
          task_id: request.taskId,
          step_id: request.stepId,
          params: resolved.params,
          status: "pending",
          outcome_note: "",
        }}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={onResolve}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Decline" }));

    await waitFor(() =>
      expect(onResolve).toHaveBeenCalledWith({
        actionRequestId: "act-review-decline",
        originTurnId: "turn-review-1",
        disposition: "declined",
      }),
    );
    expect(dialogCalls.has("service_access_review")).toBe(false);
  });

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

  it("wires connection.revoke through its typed dialog and connection report", async () => {
    await runJourney({
      action: "connection.revoke",
      rawParams: { serviceId: "legacy-service-1" },
      variant: "connection_revoke",
      normalizedParams: {
        variant: "connection_revoke",
        service_id: "legacy-service-1",
      },
      cta: "Revoke connection",
      dialogParams: { serviceId: "legacy-service-1" },
      completion: "legacy-service-1",
      resource: { connection: { serviceId: "legacy-service-1" } },
    });
  });

  it("wires provider.disconnect through its typed dialog and provider-token report", async () => {
    await runJourney({
      action: "provider.disconnect",
      rawParams: { providerId: "provider-disconnect-1" },
      variant: "provider_disconnect",
      normalizedParams: {
        variant: "provider_disconnect",
        provider_id: "provider-disconnect-1",
      },
      cta: "Disconnect provider",
      dialogParams: { providerId: "provider-disconnect-1" },
      completion: "provider-disconnect-1",
      resource: {
        providerToken: { providerId: "provider-disconnect-1" },
      },
    });
  });

  it("wires provider.set_app_credentials through its typed dialog and credentials report", async () => {
    await runJourney({
      action: "provider.set_app_credentials",
      rawParams: { providerId: "provider-credentials-1" },
      variant: "provider_set_app_credentials",
      normalizedParams: {
        variant: "provider_set_app_credentials",
        provider_id: "provider-credentials-1",
      },
      cta: "Save credentials",
      dialogParams: { providerId: "provider-credentials-1" },
      completion: "provider-credentials-1",
      resource: {
        providerCredentials: { providerId: "provider-credentials-1" },
      },
    });
  });

  it("rejects model-supplied credential fields before dialog binding", () => {
    const parsed = assistantActionRequestSchema.safeParse({
      schemaVersion: ACTION_SCHEMA_VERSION,
      actorId: "conversation-1",
      originTurnId: "turn-origin-1",
      taskId: "task-1",
      stepId: "step-1",
      actionRequestId: "act-provider-secret-injection",
      action: "provider.set_app_credentials",
      params: {
        providerId: "provider-credentials-1",
        clientSecret: "model-supplied-secret",
        confirmed: true,
      },
    });

    expect(parsed.success).toBe(false);
    expect(dialogCalls.has("provider_set_app_credentials")).toBe(false);
  });
});

const WAVE_3_4_JOURNEYS: readonly JourneyOptions[] = [
  {
    action: "node.register_token",
    rawParams: { name: "Edge node", targetOrgId: "org-1" },
    variant: "node_register_token",
    normalizedParams: {
      variant: "node_register_token",
      name: "Edge node",
      target_org_id: "org-1",
    },
    cta: "Create registration token",
    dialogParams: { name: "Edge node", targetOrgId: "org-1" },
    completion: "node-registered",
    resource: { node: { nodeId: "node-registered" } },
  },
  {
    action: "node.rotate_token",
    rawParams: { nodeId: "node-rotate" },
    variant: "node_rotate_token",
    normalizedParams: { variant: "node_rotate_token", node_id: "node-rotate" },
    cta: "Rotate node token",
    dialogParams: { nodeId: "node-rotate" },
    completion: "node-rotated",
    resource: { node: { nodeId: "node-rotated" } },
  },
  {
    action: "node.delete",
    rawParams: { nodeId: "node-delete" },
    variant: "node_delete",
    normalizedParams: { variant: "node_delete", node_id: "node-delete" },
    cta: "Delete node",
    dialogParams: { nodeId: "node-delete" },
    completion: "node-deleted",
    resource: { node: { nodeId: "node-deleted" } },
  },
  {
    action: "node.transfer",
    rawParams: { nodeId: "node-transfer", newOwnerUserId: "user-new-owner" },
    variant: "node_transfer",
    normalizedParams: {
      variant: "node_transfer",
      node_id: "node-transfer",
      new_owner_user_id: "user-new-owner",
    },
    cta: "Transfer node",
    dialogParams: { nodeId: "node-transfer", newOwnerUserId: "user-new-owner" },
    completion: "node-transferred",
    resource: { node: { nodeId: "node-transferred" } },
  },
  {
    action: "node.inject_credential",
    rawParams: {
      nodeId: "node-inject",
      serviceSlug: "github",
      injectionMethod: "header",
      fieldName: "Authorization",
      targetUrl: "https://api.github.test",
      label: "GitHub",
    },
    variant: "node_inject_credential",
    normalizedParams: {
      variant: "node_inject_credential",
      node_id: "node-inject",
      service_slug: "github",
      injection_method: "header",
      field_name: "Authorization",
      target_url: "https://api.github.test",
      label: "GitHub",
    },
    cta: "Inject credential",
    dialogParams: {
      nodeId: "node-inject",
      serviceSlug: "github",
      injectionMethod: "header",
      fieldName: "Authorization",
      targetUrl: "https://api.github.test",
      label: "GitHub",
    },
    completion: "pending-injected",
    resource: {
      pendingCredential: { pendingCredentialId: "pending-injected" },
    },
  },
  {
    action: "pending_credential.push",
    rawParams: {
      nodeId: "node-push",
      serviceSlug: "openai",
      injectionMethod: "query-param",
      fieldName: "api_key",
    },
    variant: "pending_credential_push",
    normalizedParams: {
      variant: "pending_credential_push",
      node_id: "node-push",
      service_slug: "openai",
      injection_method: "query-param",
      field_name: "api_key",
    },
    cta: "Push credential",
    dialogParams: {
      nodeId: "node-push",
      serviceSlug: "openai",
      injectionMethod: "query-param",
      fieldName: "api_key",
    },
    completion: "pending-pushed",
    resource: { pendingCredential: { pendingCredentialId: "pending-pushed" } },
  },
  {
    action: "pending_credential.cancel",
    rawParams: { nodeId: "node-cancel", pendingCredentialId: "pending-cancel" },
    variant: "pending_credential_cancel",
    normalizedParams: {
      variant: "pending_credential_cancel",
      node_id: "node-cancel",
      pending_credential_id: "pending-cancel",
    },
    cta: "Cancel credential",
    dialogParams: {
      nodeId: "node-cancel",
      pendingCredentialId: "pending-cancel",
    },
    completion: "pending-cancelled",
    resource: {
      pendingCredential: { pendingCredentialId: "pending-cancelled" },
    },
  },
  {
    action: "device.onboard",
    rawParams: {
      label: "Kitchen",
      targetOrgId: "org-1",
      defaultServiceIds: ["service-1", "service-2"],
    },
    variant: "device_onboard",
    normalizedParams: {
      variant: "device_onboard",
      label: "Kitchen",
      target_org_id: "org-1",
      default_service_ids: ["service-1", "service-2"],
    },
    cta: "Onboard device",
    dialogParams: {
      label: "Kitchen",
      targetOrgId: "org-1",
      defaultServiceIds: ["service-1", "service-2"],
    },
    completion: "device-onboarded",
    resource: { device: { deviceId: "device-onboarded" } },
  },
  {
    action: "org.create",
    rawParams: {
      displayName: "Platform",
      contactEmail: "platform@example.test",
      avatarUrl: "https://example.test/avatar.png",
    },
    variant: "org_create",
    normalizedParams: {
      variant: "org_create",
      display_name: "Platform",
      contact_email: "platform@example.test",
      avatar_url: "https://example.test/avatar.png",
    },
    cta: "Create organization",
    dialogAction: "create",
    dialogParams: {
      displayName: "Platform",
      contactEmail: "platform@example.test",
      avatarUrl: "https://example.test/avatar.png",
    },
    completion: "org-created",
    resource: { org: { orgId: "org-created" } },
  },
  {
    action: "org.update",
    rawParams: {
      orgId: "org-update",
      displayName: "Platform Ops",
      slug: "platform-ops",
    },
    variant: "org_update",
    normalizedParams: {
      variant: "org_update",
      org_id: "org-update",
      display_name: "Platform Ops",
      slug: "platform-ops",
    },
    cta: "Update organization",
    dialogAction: "update",
    dialogParams: {
      orgId: "org-update",
      displayName: "Platform Ops",
      slug: "platform-ops",
    },
    completion: "org-updated",
    resource: { org: { orgId: "org-updated" } },
  },
  {
    action: "org.delete",
    rawParams: { orgId: "org-delete" },
    variant: "org_delete",
    normalizedParams: { variant: "org_delete", org_id: "org-delete" },
    cta: "Delete organization",
    dialogAction: "delete",
    dialogParams: { orgId: "org-delete" },
    completion: "org-deleted",
    resource: { org: { orgId: "org-deleted" } },
  },
  {
    action: "org.member_add",
    rawParams: {
      orgId: "org-member",
      userId: "user-add",
      role: "member",
      allowedServiceIds: ["hidden-service"],
    },
    variant: "org_member_add",
    normalizedParams: {
      variant: "org_member_add",
      org_id: "org-member",
      user_id: "user-add",
      role: "member",
      allowed_service_ids: ["hidden-service"],
    },
    cta: "Add member",
    dialogAction: "member_add",
    dialogParams: {
      orgId: "org-member",
      userId: "user-add",
      role: "member",
      allowedServiceIds: ["hidden-service"],
    },
    completion: "org-member-added",
    resource: { org: { orgId: "org-member-added" } },
  },
  {
    action: "org.member_remove",
    rawParams: { orgId: "org-member", memberId: "member-remove" },
    variant: "org_member_remove",
    normalizedParams: {
      variant: "org_member_remove",
      org_id: "org-member",
      member_id: "member-remove",
    },
    cta: "Remove member",
    dialogAction: "member_remove",
    dialogParams: { orgId: "org-member", memberId: "member-remove" },
    completion: "org-member-removed",
    resource: { org: { orgId: "org-member-removed" } },
  },
  {
    action: "org.member_update_role",
    rawParams: { orgId: "org-member", memberId: "member-role", role: "admin" },
    variant: "org_member_update_role",
    normalizedParams: {
      variant: "org_member_update_role",
      org_id: "org-member",
      member_id: "member-role",
      role: "admin",
    },
    cta: "Change member role",
    dialogAction: "member_update_role",
    dialogParams: {
      orgId: "org-member",
      memberId: "member-role",
      role: "admin",
    },
    completion: "org-role-updated",
    resource: { org: { orgId: "org-role-updated" } },
  },
  {
    action: "org.invite",
    rawParams: {
      orgId: "org-invite",
      role: "viewer",
      allowedServiceIds: ["hidden-service"],
    },
    variant: "org_invite",
    normalizedParams: {
      variant: "org_invite",
      org_id: "org-invite",
      role: "viewer",
      allowed_service_ids: ["hidden-service"],
    },
    cta: "Create invite",
    dialogAction: "invite",
    dialogParams: {
      orgId: "org-invite",
      role: "viewer",
      allowedServiceIds: ["hidden-service"],
    },
    completion: "org-invited",
    resource: { org: { orgId: "org-invited" } },
  },
  {
    action: "org.set_primary",
    rawParams: { orgId: "org-primary" },
    variant: "org_set_primary",
    normalizedParams: { variant: "org_set_primary", org_id: "org-primary" },
    cta: "Set primary",
    dialogAction: "set_primary",
    dialogParams: { orgId: "org-primary" },
    completion: "org-primary-set",
    resource: { org: { orgId: "org-primary-set" } },
  },
  {
    action: "account.profile_update",
    rawParams: {
      displayName: "Ada",
      avatarUrl: "https://example.test/ada.png",
    },
    variant: "account_profile_update",
    normalizedParams: {
      variant: "account_profile_update",
      display_name: "Ada",
      avatar_url: "https://example.test/ada.png",
    },
    cta: "Update profile",
    dialogParams: {
      displayName: "Ada",
      avatarUrl: "https://example.test/ada.png",
    },
    completion: "user-profile-updated",
    resource: { account: { userId: "user-profile-updated" } },
  },
  {
    action: "account.revoke_consent",
    rawParams: { clientId: "client-revoke" },
    variant: "account_revoke_consent",
    normalizedParams: {
      variant: "account_revoke_consent",
      client_id: "client-revoke",
    },
    cta: "Revoke consent",
    dialogParams: { clientId: "client-revoke" },
    completion: "user-consent-revoked",
    resource: { account: { userId: "user-consent-revoked" } },
  },
  {
    action: "account.delete",
    rawParams: {},
    variant: "account_delete",
    normalizedParams: { variant: "account_delete" },
    cta: "Delete account",
    completion: "user-deleted",
    resource: { account: { userId: "user-deleted" } },
  },
  {
    action: "account.mfa_setup",
    rawParams: {},
    variant: "account_mfa_setup",
    normalizedParams: { variant: "account_mfa_setup" },
    cta: "Set up MFA",
    completion: "user-mfa",
    resource: { account: { userId: "user-mfa" } },
  },
  {
    action: "approval.configure",
    rawParams: { serviceId: "service-configure" },
    variant: "approval_configure",
    normalizedParams: {
      variant: "approval_configure",
      service_id: "service-configure",
    },
    cta: "Configure approvals",
    dialogParams: { serviceId: "service-configure" },
    completion: "service-configured",
    resource: { approvalConfig: { serviceId: "service-configured" } },
  },
  {
    action: "approval.enable",
    rawParams: { serviceId: "service-enable" },
    variant: "approval_enable",
    normalizedParams: {
      variant: "approval_enable",
      service_id: "service-enable",
    },
    cta: "Enable approvals",
    dialogParams: { serviceId: "service-enable" },
    completion: "service-enabled",
    resource: { approvalConfig: { serviceId: "service-enabled" } },
  },
  {
    action: "approval.disable",
    rawParams: { serviceId: "service-disable" },
    variant: "approval_disable",
    normalizedParams: {
      variant: "approval_disable",
      service_id: "service-disable",
    },
    cta: "Disable approvals",
    dialogParams: { serviceId: "service-disable" },
    completion: "service-disabled",
    resource: { approvalConfig: { serviceId: "service-disabled" } },
  },
  {
    action: "approval.revoke_grant",
    rawParams: { grantId: "grant-revoke" },
    variant: "approval_revoke_grant",
    normalizedParams: {
      variant: "approval_revoke_grant",
      grant_id: "grant-revoke",
    },
    cta: "Revoke grant",
    dialogParams: { grantId: "grant-revoke" },
    completion: "grant-revoked",
    resource: { grant: { grantId: "grant-revoked" } },
  },
  {
    action: "notifications.update",
    rawParams: {},
    variant: "notifications_update",
    normalizedParams: { variant: "notifications_update" },
    cta: "Update notifications",
    dialogAction: "update",
    dialogParams: {},
    completion: "notifications-updated",
    resource: { notificationBinding: { bindingId: "notifications-updated" } },
  },
  {
    action: "notifications.telegram_link",
    rawParams: {},
    variant: "notifications_telegram_link",
    normalizedParams: { variant: "notifications_telegram_link" },
    cta: "Link Telegram",
    dialogAction: "telegram_link",
    dialogParams: {},
    completion: "telegram-linked",
    resource: { notificationBinding: { bindingId: "telegram-linked" } },
  },
  {
    action: "notifications.telegram_disconnect",
    rawParams: {},
    variant: "notifications_telegram_disconnect",
    normalizedParams: { variant: "notifications_telegram_disconnect" },
    cta: "Disconnect Telegram",
    dialogAction: "telegram_disconnect",
    dialogParams: {},
    completion: "telegram-disconnected",
    resource: { notificationBinding: { bindingId: "telegram-disconnected" } },
  },
  {
    action: "service_account.create",
    rawParams: {
      name: "Deploy agent",
      description: "Production deploys",
      targetOrgId: "org-production",
    },
    variant: "service_account_create",
    normalizedParams: {
      variant: "service_account_create",
      name: "Deploy agent",
      description: "Production deploys",
      target_org_id: "org-production",
    },
    cta: "Create service account",
    dialogAction: "create",
    dialogParams: {
      name: "Deploy agent",
      description: "Production deploys",
      targetOrgId: "org-production",
    },
    completion: "service-account-created",
    resource: {
      serviceAccount: { serviceAccountId: "service-account-created" },
    },
  },
  {
    action: "service_account.update",
    rawParams: {
      serviceAccountId: "service-account-update",
      name: "Deploy agent v2",
    },
    variant: "service_account_update",
    normalizedParams: {
      variant: "service_account_update",
      service_account_id: "service-account-update",
      name: "Deploy agent v2",
    },
    cta: "Update service account",
    dialogAction: "update",
    dialogParams: {
      serviceAccountId: "service-account-update",
      name: "Deploy agent v2",
    },
    completion: "service-account-updated",
    resource: {
      serviceAccount: { serviceAccountId: "service-account-updated" },
    },
  },
  {
    action: "service_account.delete",
    rawParams: { serviceAccountId: "service-account-delete" },
    variant: "service_account_delete",
    normalizedParams: {
      variant: "service_account_delete",
      service_account_id: "service-account-delete",
    },
    cta: "Delete service account",
    dialogAction: "delete",
    dialogParams: { serviceAccountId: "service-account-delete" },
    completion: "service-account-deleted",
    resource: {
      serviceAccount: { serviceAccountId: "service-account-deleted" },
    },
  },
  {
    action: "service_account.rotate_secret",
    rawParams: { serviceAccountId: "service-account-rotate" },
    variant: "service_account_rotate_secret",
    normalizedParams: {
      variant: "service_account_rotate_secret",
      service_account_id: "service-account-rotate",
    },
    cta: "Rotate service-account secret",
    dialogAction: "rotate_secret",
    dialogParams: { serviceAccountId: "service-account-rotate" },
    completion: "service-account-rotated",
    resource: {
      serviceAccount: { serviceAccountId: "service-account-rotated" },
    },
  },
  {
    action: "service_account.revoke_tokens",
    rawParams: { serviceAccountId: "service-account-revoke" },
    variant: "service_account_revoke_tokens",
    normalizedParams: {
      variant: "service_account_revoke_tokens",
      service_account_id: "service-account-revoke",
    },
    cta: "Revoke service-account tokens",
    dialogAction: "revoke_tokens",
    dialogParams: { serviceAccountId: "service-account-revoke" },
    completion: "service-account-revoked",
    resource: {
      serviceAccount: { serviceAccountId: "service-account-revoked" },
    },
  },
  {
    action: "developer_app.create",
    rawParams: {
      name: "Console",
      redirectUris: ["https://console.example.test/callback"],
    },
    variant: "developer_app_create",
    normalizedParams: {
      variant: "developer_app_create",
      name: "Console",
      redirect_uris: ["https://console.example.test/callback"],
    },
    cta: "Create developer app",
    dialogAction: "create",
    dialogParams: {
      name: "Console",
      redirectUris: ["https://console.example.test/callback"],
    },
    completion: "developer-app-created",
    resource: { developerApp: { clientId: "developer-app-created" } },
  },
  {
    action: "developer_app.update",
    rawParams: {
      clientId: "developer-app-update",
      name: "Console v2",
      redirectUris: ["https://console.example.test/oauth/callback"],
    },
    variant: "developer_app_update",
    normalizedParams: {
      variant: "developer_app_update",
      client_id: "developer-app-update",
      name: "Console v2",
      redirect_uris: ["https://console.example.test/oauth/callback"],
    },
    cta: "Update developer app",
    dialogAction: "update",
    dialogParams: {
      clientId: "developer-app-update",
      name: "Console v2",
      redirectUris: ["https://console.example.test/oauth/callback"],
    },
    completion: "developer-app-updated",
    resource: { developerApp: { clientId: "developer-app-updated" } },
  },
  {
    action: "developer_app.delete",
    rawParams: { clientId: "developer-app-delete" },
    variant: "developer_app_delete",
    normalizedParams: {
      variant: "developer_app_delete",
      client_id: "developer-app-delete",
    },
    cta: "Delete developer app",
    dialogAction: "delete",
    dialogParams: { clientId: "developer-app-delete" },
    completion: "developer-app-deleted",
    resource: { developerApp: { clientId: "developer-app-deleted" } },
  },
  {
    action: "developer_app.rotate_secret",
    rawParams: { clientId: "developer-app-rotate" },
    variant: "developer_app_rotate_secret",
    normalizedParams: {
      variant: "developer_app_rotate_secret",
      client_id: "developer-app-rotate",
    },
    cta: "Rotate developer-app secret",
    dialogAction: "rotate_secret",
    dialogParams: { clientId: "developer-app-rotate" },
    completion: "developer-app-rotated",
    resource: { developerApp: { clientId: "developer-app-rotated" } },
  },
  {
    action: "external_key.add_gcp_service_account",
    rawParams: { label: "GCP production", targetOrgId: "org-production" },
    variant: "external_key_add_gcp_service_account",
    normalizedParams: {
      variant: "external_key_add_gcp_service_account",
      label: "GCP production",
      target_org_id: "org-production",
    },
    cta: "Add GCP service account",
    dialogAction: "external_key.add_gcp_service_account",
    dialogParams: { label: "GCP production", targetOrgId: "org-production" },
    completion: "external-key-gcp",
    resource: { externalKey: { externalKeyId: "external-key-gcp" } },
  },
  {
    action: "openclaw.connect",
    rawParams: { gatewayUrl: "https://openclaw.example.test" },
    variant: "openclaw_connect",
    normalizedParams: {
      variant: "openclaw_connect",
      gateway_url: "https://openclaw.example.test",
    },
    cta: "Connect OpenClaw",
    dialogAction: "openclaw.connect",
    dialogParams: { gatewayUrl: "https://openclaw.example.test" },
    completion: "openclaw-service",
    resource: { userService: { userServiceId: "openclaw-service" } },
  },
];

describe("Wave-3 and Wave-4 action card wiring", () => {
  it.each(WAVE_3_4_JOURNEYS)(
    "wires $action through $variant and reports its resource",
    async (journey) => {
      await runJourney(journey);
    },
  );
});
