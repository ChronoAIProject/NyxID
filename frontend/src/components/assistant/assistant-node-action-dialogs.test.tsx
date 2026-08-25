import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { AssistantDeviceOnboardDialog } from "./assistant-device-onboard-dialog";
import { AssistantNodeDeleteDialog } from "./assistant-node-delete-dialog";
import { AssistantNodeInjectCredentialDialog } from "./assistant-node-inject-credential-dialog";
import { AssistantNodeRegisterTokenDialog } from "./assistant-node-register-token-dialog";
import { AssistantNodeRotateTokenDialog } from "./assistant-node-rotate-token-dialog";
import { AssistantNodeTransferDialog } from "./assistant-node-transfer-dialog";
import { AssistantPendingCredentialCancelDialog } from "./assistant-pending-credential-cancel-dialog";
import { AssistantPendingCredentialPushDialog } from "./assistant-pending-credential-push-dialog";

const { mockGet, mockPost, mockToDataURL } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockToDataURL: vi.fn(),
}));

vi.mock("@/lib/api-client", () => {
  class ApiError extends Error {
    readonly status: number;
    readonly errorCode: number;
    readonly errorResponse: {
      error: string;
      error_code: number;
      message: string;
    };

    constructor(
      status: number,
      response: { error: string; error_code: number; message: string },
    ) {
      super(response.message);
      this.status = status;
      this.errorCode = response.error_code;
      this.errorResponse = response;
    }
  }
  return { api: { get: mockGet, post: mockPost }, ApiError };
});

vi.mock("qrcode", () => ({
  default: { toDataURL: mockToDataURL },
}));

const NODE_ID = "11111111-1111-4111-8111-111111111111";
const OWNER_ID = "22222222-2222-4222-8222-222222222222";
const NEW_OWNER_ID = "33333333-3333-4333-8333-333333333333";
const PENDING_ID = "44444444-4444-4444-8444-444444444444";
const DEVICE_ID = "55555555-5555-4555-8555-555555555555";
const REQUESTED_AT = "2026-08-25T00:00:00Z";

function nodeEvidence(overrides: Readonly<Record<string, unknown>> = {}) {
  return {
    id: NODE_ID,
    owner_user_id: OWNER_ID,
    lifecycle: "active",
    is_active: true,
    state_version: 4,
    access_revision: 2,
    created_at: "2026-08-20T00:00:00Z",
    updated_at: "2026-08-24T00:00:00Z",
    registration_expires_at: null,
    ...overrides,
  };
}

function pendingEvidence(overrides: Readonly<Record<string, unknown>> = {}) {
  return {
    id: PENDING_ID,
    node_id: NODE_ID,
    owner_user_id: OWNER_ID,
    remote_state: "queued",
    is_active: true,
    created_at: "2026-08-25T00:00:00Z",
    expires_at: "2026-08-26T00:00:00Z",
    consumed_at: null,
    declined_at: null,
    state_version: 1,
    ...overrides,
  };
}

beforeEach(() => {
  mockGet.mockReset();
  mockPost.mockReset();
  mockToDataURL.mockReset();
  mockToDataURL.mockResolvedValue("data:image/png;base64,qr");
});

describe("AssistantNodeRegisterTokenDialog", () => {
  it("creates a registration token, verifies canonical evidence, and shows it once", async () => {
    mockPost.mockResolvedValue({
      resource: { nodeId: NODE_ID },
      replayed: false,
      requestedAt: REQUESTED_AT,
      oneTimeMaterial: "delivered",
      registrationToken: "nyx_nreg_one_time_value",
      expiresAt: "2026-08-25T01:00:00Z",
    });
    mockGet.mockResolvedValue(
      nodeEvidence({
        lifecycle: "registration_pending",
        owner_user_id: OWNER_ID,
      }),
    );
    const onComplete = vi.fn();
    render(
      <AssistantNodeRegisterTokenDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-register"
        params={{ name: "Edge node", targetOrgId: OWNER_ID }}
        onComplete={onComplete}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Create token" }));

    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/nodes/register-token",
      {
        actionRequestId: "act-register",
        name: "Edge node",
        targetOrgId: OWNER_ID,
      },
    );
    expect(mockGet).toHaveBeenCalledWith(
      `/assistant/actions/nodes/${NODE_ID}/authorization`,
    );
    expect(
      await screen.findByDisplayValue("nyx_nreg_one_time_value"),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "I have saved it" }),
    );
    expect(onComplete).toHaveBeenCalledWith(NODE_ID);
  });

  it("states plainly when replayed one-time material was unavailable", async () => {
    mockPost.mockResolvedValue({
      resource: { nodeId: NODE_ID },
      replayed: true,
      requestedAt: REQUESTED_AT,
      oneTimeMaterial: "unavailable",
    });
    mockGet.mockResolvedValue(
      nodeEvidence({ lifecycle: "registration_pending" }),
    );
    render(
      <AssistantNodeRegisterTokenDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-register-replay"
        params={{ name: "Edge node" }}
        onComplete={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Create token" }));

    expect(
      await screen.findByText(/one-time token was not captured/i),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Acknowledge" })).toBeEnabled();
    expect(screen.queryByLabelText("One-time registration token")).toBeNull();
  });
});

describe("AssistantNodeRotateTokenDialog", () => {
  it("reads both revisions and shows the rotated credentials once", async () => {
    mockGet.mockResolvedValueOnce(nodeEvidence()).mockResolvedValueOnce(
      nodeEvidence({
        state_version: 5,
        access_revision: 3,
        updated_at: "2026-08-25T00:01:00Z",
      }),
    );
    mockPost.mockResolvedValue({
      resource: { nodeId: NODE_ID },
      replayed: false,
      requestedAt: REQUESTED_AT,
      oneTimeMaterial: "delivered",
      authToken: "nyx_nauth_one_time_value",
      signingSecret: "one-time-signing-secret",
    });
    const onComplete = vi.fn();
    render(
      <AssistantNodeRotateTokenDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-rotate"
        params={{ nodeId: NODE_ID }}
        onComplete={onComplete}
      />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Rotate credentials" }),
    );

    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/nodes/rotate-token",
      {
        actionRequestId: "act-rotate",
        nodeId: NODE_ID,
      },
    );
    expect(mockGet).toHaveBeenNthCalledWith(
      1,
      `/assistant/actions/nodes/${NODE_ID}/authorization`,
    );
    expect(mockGet).toHaveBeenNthCalledWith(
      2,
      `/assistant/actions/nodes/${NODE_ID}/authorization`,
    );
    expect(
      await screen.findByDisplayValue("nyx_nauth_one_time_value"),
    ).toBeInTheDocument();
    expect(
      screen.getByDisplayValue("one-time-signing-secret"),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "I have saved it" }),
    );
    expect(onComplete).toHaveBeenCalledWith(NODE_ID);
  });
});

describe("AssistantNodeDeleteDialog", () => {
  it("requires confirmation and derives expectedStateVersion from evidence", async () => {
    mockGet
      .mockResolvedValueOnce(nodeEvidence({ state_version: 9 }))
      .mockRejectedValueOnce(
        new ApiError(404, {
          error: "not_found",
          error_code: 8000,
          message: "node not found",
        }),
      );
    mockPost.mockResolvedValue({
      resource: { nodeId: NODE_ID },
      replayed: false,
      requestedAt: REQUESTED_AT,
    });
    const onComplete = vi.fn();
    render(
      <AssistantNodeDeleteDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-delete"
        params={{ nodeId: NODE_ID }}
        onComplete={onComplete}
      />,
    );

    const deleteButton = screen.getByRole("button", { name: "Delete node" });
    expect(deleteButton).toBeDisabled();
    await userEvent.click(
      screen.getByRole("checkbox", { name: /removes node routing/i }),
    );
    await userEvent.click(deleteButton);

    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/nodes/delete", {
      actionRequestId: "act-delete",
      nodeId: NODE_ID,
      expectedStateVersion: 9,
    });
    expect(
      await screen.findByText("Credential node deleted"),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(NODE_ID);
  });
});

describe("AssistantNodeTransferDialog", () => {
  it("requires confirmation and verifies the evidence-derived ownership transition", async () => {
    mockGet
      .mockResolvedValueOnce(nodeEvidence({ state_version: 6 }))
      .mockResolvedValueOnce(
        nodeEvidence({
          owner_user_id: NEW_OWNER_ID,
          state_version: 7,
          updated_at: "2026-08-25T00:02:00Z",
        }),
      );
    mockPost.mockResolvedValue({
      resource: { nodeId: NODE_ID },
      replayed: false,
      requestedAt: REQUESTED_AT,
    });
    const onComplete = vi.fn();
    render(
      <AssistantNodeTransferDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-transfer"
        params={{ nodeId: NODE_ID, newOwnerUserId: NEW_OWNER_ID }}
        onComplete={onComplete}
      />,
    );

    const transferButton = screen.getByRole("button", {
      name: "Transfer node",
    });
    expect(transferButton).toBeDisabled();
    await userEvent.click(
      screen.getByRole("checkbox", {
        name: /current owner will lose control/i,
      }),
    );
    await userEvent.click(transferButton);

    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/nodes/transfer", {
      actionRequestId: "act-transfer",
      nodeId: NODE_ID,
      newOwnerUserId: NEW_OWNER_ID,
      expectedStateVersion: 6,
    });
    expect(
      await screen.findByText("Credential node transferred"),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(NODE_ID);
  });

  it("completes a non-replayed transfer when the node leaves the actor's read scope", async () => {
    mockGet
      .mockResolvedValueOnce(nodeEvidence({ state_version: 6 }))
      .mockRejectedValueOnce(
        new ApiError(404, {
          error: "not_found",
          error_code: 8000,
          message: "node not found",
        }),
      );
    mockPost.mockResolvedValue({
      resource: { nodeId: NODE_ID },
      replayed: false,
      requestedAt: REQUESTED_AT,
    });
    const onComplete = vi.fn();
    render(
      <AssistantNodeTransferDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-transfer-out-of-scope"
        params={{ nodeId: NODE_ID, newOwnerUserId: NEW_OWNER_ID }}
        onComplete={onComplete}
      />,
    );

    await userEvent.click(
      screen.getByRole("checkbox", {
        name: /current owner will lose control/i,
      }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Transfer node" }),
    );

    expect(
      await screen.findByText(/ownership moved out of its access scope/i),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(NODE_ID);
  });

  it("completes a replayed transfer when the node is no longer readable", async () => {
    mockGet
      .mockResolvedValueOnce(nodeEvidence({ state_version: 6 }))
      .mockRejectedValueOnce(
        new ApiError(404, {
          error: "not_found",
          error_code: 8000,
          message: "node not found",
        }),
      );
    mockPost.mockResolvedValue({
      resource: { nodeId: NODE_ID },
      replayed: true,
      requestedAt: REQUESTED_AT,
    });
    const onComplete = vi.fn();
    render(
      <AssistantNodeTransferDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-transfer-replayed"
        params={{ nodeId: NODE_ID, newOwnerUserId: NEW_OWNER_ID }}
        onComplete={onComplete}
      />,
    );

    await userEvent.click(
      screen.getByRole("checkbox", {
        name: /current owner will lose control/i,
      }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Transfer node" }),
    );

    expect(
      await screen.findByText(/ownership moved out of its access scope/i),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(NODE_ID);
  });
});

describe("AssistantNodeInjectCredentialDialog", () => {
  it("verifies the node, posts reviewed fields, and reads pending evidence", async () => {
    mockGet
      .mockResolvedValueOnce(nodeEvidence())
      .mockResolvedValueOnce(pendingEvidence());
    mockPost.mockResolvedValue({
      resource: { pendingCredentialId: PENDING_ID },
      replayed: false,
      requestedAt: REQUESTED_AT,
    });
    const onComplete = vi.fn();
    render(
      <AssistantNodeInjectCredentialDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-inject"
        params={{
          nodeId: NODE_ID,
          serviceSlug: "github",
          injectionMethod: "header",
          fieldName: "Authorization",
          targetUrl: "https://api.github.test",
          label: "GitHub production",
        }}
        onComplete={onComplete}
      />,
    );

    await userEvent.click(
      screen.getByRole("combobox", { name: "Injection method" }),
    );
    await userEvent.click(
      await screen.findByRole("option", { name: "Query parameter" }),
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Inject credential" }),
    );

    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/nodes/inject-credential",
      {
        actionRequestId: "act-inject",
        nodeId: NODE_ID,
        serviceSlug: "github",
        injectionMethod: "query-param",
        fieldName: "Authorization",
        targetUrl: "https://api.github.test",
        label: "GitHub production",
      },
    );
    expect(mockGet).toHaveBeenNthCalledWith(
      1,
      `/assistant/actions/nodes/${NODE_ID}/authorization`,
    );
    expect(mockGet).toHaveBeenNthCalledWith(
      2,
      `/assistant/actions/nodes/${NODE_ID}/pending/${PENDING_ID}/authorization`,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(PENDING_ID);
  });
});

describe("AssistantPendingCredentialPushDialog", () => {
  it("uses the push effect while retaining the canonical evidence checks", async () => {
    mockGet
      .mockResolvedValueOnce(nodeEvidence())
      .mockResolvedValueOnce(pendingEvidence());
    mockPost.mockResolvedValue({
      resource: { pendingCredentialId: PENDING_ID },
      replayed: false,
      requestedAt: REQUESTED_AT,
    });
    const onComplete = vi.fn();
    render(
      <AssistantPendingCredentialPushDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-push"
        params={{
          nodeId: NODE_ID,
          serviceSlug: "openai",
          injectionMethod: "query-param",
          fieldName: "api_key",
        }}
        onComplete={onComplete}
      />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Push credential" }),
    );

    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/nodes/pending-credential-push",
      {
        actionRequestId: "act-push",
        nodeId: NODE_ID,
        serviceSlug: "openai",
        injectionMethod: "query-param",
        fieldName: "api_key",
        targetUrl: undefined,
        label: undefined,
      },
    );
    await userEvent.click(await screen.findByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(PENDING_ID);
  });

  it("rejects a secret-shaped edited label before any request", async () => {
    render(
      <AssistantPendingCredentialPushDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-push-secret-label"
        params={{
          nodeId: NODE_ID,
          serviceSlug: "openai",
          injectionMethod: "header",
          fieldName: "Authorization",
        }}
        onComplete={vi.fn()}
      />,
    );

    await userEvent.type(
      screen.getByLabelText("Label (optional)"),
      "nyxid_ag_1234567890abcdef",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Push credential" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Labels cannot contain secret-shaped values.",
    );
    expect(mockGet).not.toHaveBeenCalled();
    expect(mockPost).not.toHaveBeenCalled();
  });
});

describe("AssistantPendingCredentialCancelDialog", () => {
  it("requires confirmation and verifies a terminal declined projection", async () => {
    mockGet.mockResolvedValueOnce(pendingEvidence()).mockResolvedValueOnce(
      pendingEvidence({
        remote_state: "declined",
        is_active: false,
        declined_at: "2026-08-25T00:03:00Z",
      }),
    );
    mockPost.mockResolvedValue({
      resource: { pendingCredentialId: PENDING_ID },
      replayed: false,
      requestedAt: REQUESTED_AT,
    });
    const onComplete = vi.fn();
    render(
      <AssistantPendingCredentialCancelDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-cancel"
        params={{ nodeId: NODE_ID, pendingCredentialId: PENDING_ID }}
        onComplete={onComplete}
      />,
    );

    const cancelButton = screen.getByRole("button", { name: "Cancel request" });
    expect(cancelButton).toBeDisabled();
    await userEvent.click(
      screen.getByRole("checkbox", { name: /cannot be consumed/i }),
    );
    await userEvent.click(cancelButton);

    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/nodes/pending-credential-cancel",
      {
        actionRequestId: "act-cancel",
        nodeId: NODE_ID,
        pendingCredentialId: PENDING_ID,
      },
    );
    await userEvent.click(await screen.findByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(PENDING_ID);
  });
});

describe("AssistantDeviceOnboardDialog", () => {
  it("creates and renders the one-time onboarding QR after canonical evidence", async () => {
    mockPost.mockResolvedValue({
      resource: { deviceId: DEVICE_ID },
      replayed: false,
      requestedAt: REQUESTED_AT,
      oneTimeMaterial: "delivered",
      qrPayload: "nyx-device-provisioning-payload",
      expiresAt: "2026-08-25T01:00:00Z",
    });
    mockGet.mockResolvedValue({
      id: DEVICE_ID,
      owner_user_id: OWNER_ID,
      used: false,
      redeemed_node_id: null,
      created_at: "2026-08-25T00:00:00Z",
      expires_at: "2026-08-25T01:00:00Z",
      state_version: 1,
    });
    const onComplete = vi.fn();
    render(
      <AssistantDeviceOnboardDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-device"
        params={{
          label: "Kitchen",
          targetOrgId: OWNER_ID,
          defaultServiceIds: ["service-1", "service-2"],
        }}
        onComplete={onComplete}
      />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Create onboarding package" }),
    );

    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/nodes/device-onboard",
      {
        actionRequestId: "act-device",
        label: "Kitchen",
        targetOrgId: OWNER_ID,
        defaultServiceIds: ["service-1", "service-2"],
      },
    );
    expect(mockGet).toHaveBeenCalledWith(
      `/assistant/actions/nodes/devices/${DEVICE_ID}/authorization`,
    );
    expect(mockToDataURL).toHaveBeenCalledWith(
      "nyx-device-provisioning-payload",
      { errorCorrectionLevel: "M", margin: 2, width: 256 },
    );
    expect(
      await screen.findByRole("img", {
        name: "One-time device onboarding QR code",
      }),
    ).toHaveAttribute("src", "data:image/png;base64,qr");
    expect(
      screen.getByDisplayValue("nyx-device-provisioning-payload"),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "I have saved it" }),
    );
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith(DEVICE_ID));
  });
});
