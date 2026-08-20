import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantServiceRotateCredentialDialog } from "./assistant-service-rotate-credential-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error {
    readonly status: number;
    constructor(status: number) {
      super(`HTTP ${String(status)}`);
      this.status = status;
    }
  },
}));

const SERVICE_ID = "11111111-1111-4111-8111-111111111111";

beforeEach(() => {
  mockGet.mockReset();
  mockPost.mockReset();
});

describe("AssistantServiceRotateCredentialDialog", () => {
  it("never surfaces credential material from the effect or evidence", async () => {
    mockPost.mockResolvedValue({
      resource: { userServiceId: SERVICE_ID },
      replayed: false,
    });
    mockGet.mockResolvedValueOnce({
      id: SERVICE_ID,
      api_key_id: "44444444-4444-4444-8444-444444444444",
      is_active: true,
      status: "active",
      connection_status: null,
      granted_scopes: null,
      last_authorized_at: null,
      node_id: null,
      rotation_predecessor_id: null,
      state_version: 1,
      updated_at: "2026-08-19T00:00:00Z",
    });
    mockGet.mockResolvedValueOnce({
      id: SERVICE_ID,
      api_key_id: "66666666-6666-4666-8666-666666666666",
      is_active: true,
      status: "active",
      connection_status: null,
      granted_scopes: null,
      last_authorized_at: null,
      node_id: null,
      rotation_predecessor_id: "44444444-4444-4444-8444-444444444444",
      state_version: 2,
      updated_at: "2026-08-20T00:00:00Z",
    });
    const onComplete = vi.fn();
    render(
      <AssistantServiceRotateCredentialDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-rotate"
        params={{ userServiceId: SERVICE_ID }}
        onComplete={onComplete}
      />,
    );

    await userEvent.type(
      screen.getByLabelText("Replacement credential"),
      "sk-live-must-not-leak",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Rotate credential" }),
    );
    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/services/rotate-credential",
      {
        actionRequestId: "act-rotate",
        userServiceId: SERVICE_ID,
        credential: "sk-live-must-not-leak",
      },
    );
    expect(screen.queryByDisplayValue("sk-live-must-not-leak")).toBeNull();
    expect(
      screen.queryByText("sk-live-must-not-leak"),
    ).not.toBeInTheDocument();
    expect(
      await screen.findByText(
        "Rotation lineage verified. No credential material returned.",
      ),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Report service" }),
    );
    expect(onComplete).toHaveBeenCalledWith(SERVICE_ID);
  });

  it("rejects an effect response that includes credential material", async () => {
    mockGet.mockResolvedValue({
      id: SERVICE_ID,
      api_key_id: "44444444-4444-4444-8444-444444444444",
      is_active: true,
      status: "active",
      connection_status: null,
      granted_scopes: null,
      last_authorized_at: null,
      node_id: null,
      rotation_predecessor_id: null,
      state_version: 1,
      updated_at: "2026-08-19T00:00:00Z",
    });
    mockPost.mockResolvedValue({
      resource: { userServiceId: SERVICE_ID },
      replayed: false,
      credential: "sk-leaked",
    });
    render(
      <AssistantServiceRotateCredentialDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-rotate"
        params={{ userServiceId: SERVICE_ID }}
        onComplete={vi.fn()}
      />,
    );
    await userEvent.type(
      screen.getByLabelText("Replacement credential"),
      "sk-live-must-not-leak",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Rotate credential" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "secret-bearing verification data",
    );
    expect(mockGet).toHaveBeenCalledTimes(1);
    expect(mockPost).toHaveBeenCalledTimes(1);
  });

  it("rejects a stale pre-rotation projection after the effect", async () => {
    const stale = {
      id: SERVICE_ID,
      api_key_id: "44444444-4444-4444-8444-444444444444",
      is_active: true,
      status: "active",
      connection_status: null,
      granted_scopes: null,
      last_authorized_at: null,
      node_id: null,
      rotation_predecessor_id: null,
      state_version: 1,
      updated_at: "2026-08-19T00:00:00Z",
    };
    mockGet.mockResolvedValue(stale);
    mockPost.mockResolvedValue({
      resource: { userServiceId: SERVICE_ID },
      replayed: false,
    });
    render(
      <AssistantServiceRotateCredentialDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-rotate-stale"
        params={{ userServiceId: SERVICE_ID }}
        onComplete={vi.fn()}
      />,
    );

    await userEvent.type(
      screen.getByLabelText("Replacement credential"),
      "replacement",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Rotate credential" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "expected state advance",
    );
    expect(
      screen.getByRole("button", { name: "Report service" }),
    ).toBeDisabled();
  });
});
