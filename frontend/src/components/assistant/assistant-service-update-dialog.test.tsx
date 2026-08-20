import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantServiceUpdateDialog } from "./assistant-service-update-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error {
    readonly status: number;
    readonly errorCode: number;
    constructor(status: number, message = `HTTP ${String(status)}`) {
      super(message);
      this.status = status;
      this.errorCode = status;
    }
  },
}));

const SERVICE_ID = "11111111-1111-4111-8111-111111111111";

function evidence(overrides: Record<string, unknown> = {}) {
  return {
    id: SERVICE_ID,
    api_key_id: "22222222-2222-4222-8222-222222222222",
    is_active: true,
    status: "active",
    connection_status: null,
    granted_scopes: null,
    last_authorized_at: null,
    node_id: null,
    state_version: 2,
    updated_at: "2026-08-20T00:00:00Z",
    rotation_predecessor_id: null,
    ...overrides,
  };
}

function renderDialog(onComplete = vi.fn()) {
  return render(
    <AssistantServiceUpdateDialog
      open
      onOpenChange={vi.fn()}
      actionRequestId="act-update"
      params={{
        userServiceId: SERVICE_ID,
        name: "  Example  ",
        endpointUrl: "https://api.example.com",
      }}
      onComplete={onComplete}
    />,
  );
}

beforeEach(() => {
  mockGet.mockReset();
  mockPost.mockReset();
});

describe("AssistantServiceUpdateDialog", () => {
  it("normalizes fields, posts the effect, and reports after evidence read-back", async () => {
    mockPost.mockResolvedValue({
      resource: { userServiceId: SERVICE_ID },
      replayed: false,
    });
    mockGet.mockResolvedValue(evidence());
    const onComplete = vi.fn();
    renderDialog(onComplete);

    fireEvent.click(screen.getByRole("button", { name: "Update service" }));
    fireEvent.click(screen.getByRole("button", { name: "Update service" }));

    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/services/update",
      {
        actionRequestId: "act-update",
        userServiceId: SERVICE_ID,
        name: "Example",
        endpointUrl: "https://api.example.com",
      },
    );
    expect(mockGet).toHaveBeenCalledWith(
      `/keys/${SERVICE_ID}/authorization`,
    );
    expect(
      await screen.findByText("Authorization evidence verified."),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Report service" }),
    );
    expect(onComplete).toHaveBeenCalledWith(SERVICE_ID);
  });

  it("rejects secret-bearing authorization evidence", async () => {
    mockPost.mockResolvedValue({
      resource: { userServiceId: SERVICE_ID },
      replayed: false,
    });
    mockGet.mockResolvedValue(
      evidence({ ws_frame_injections: [{ template: "Bearer secret" }] }),
    );
    renderDialog();
    await userEvent.click(
      screen.getByRole("button", { name: "Update service" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "secret-bearing verification data",
    );
    expect(
      screen.getByRole("button", { name: "Report service" }),
    ).toBeDisabled();
  });
});
