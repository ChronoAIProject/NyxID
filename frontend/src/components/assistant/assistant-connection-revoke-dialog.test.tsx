import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantConnectionRevokeDialog } from "./assistant-connection-revoke-dialog";

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

describe("AssistantConnectionRevokeDialog", () => {
  it("posts explicit confirmation and reports after one evidence advance", async () => {
    mockGet
      .mockResolvedValueOnce({
        service_id: SERVICE_ID,
        is_active: true,
        state_version: 1,
        updated_at: "2026-08-25T00:00:00Z",
      })
      .mockResolvedValueOnce({
        service_id: SERVICE_ID,
        is_active: false,
        state_version: 2,
        updated_at: "2026-08-25T00:00:01Z",
      });
    mockPost.mockResolvedValue({
      resource: { serviceId: SERVICE_ID },
      replayed: false,
      oneTimeMaterial: "unavailable",
    });
    const onComplete = vi.fn();
    render(
      <AssistantConnectionRevokeDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-connection-revoke"
        params={{ serviceId: SERVICE_ID }}
        onComplete={onComplete}
      />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Revoke connection" }),
    );
    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/providers/connection-revoke",
      {
        actionRequestId: "act-connection-revoke",
        serviceId: SERVICE_ID,
        expectedStateVersion: 1,
        confirmed: true,
      },
    );
    expect(await screen.findByText("Revocation evidence verified.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(SERVICE_ID);
  });

  it("blocks reporting when an evidence read contains a credential field", async () => {
    mockGet
      .mockResolvedValueOnce({
        service_id: SERVICE_ID,
        is_active: true,
        state_version: 1,
        updated_at: "2026-08-25T00:00:00Z",
      })
      .mockResolvedValueOnce({
        service_id: SERVICE_ID,
        is_active: false,
        state_version: 2,
        updated_at: "2026-08-25T00:00:01Z",
        credential: "must-not-return",
      });
    mockPost.mockResolvedValue({
      resource: { serviceId: SERVICE_ID },
      replayed: false,
    });
    render(
      <AssistantConnectionRevokeDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-connection-poison"
        params={{ serviceId: SERVICE_ID }}
        onComplete={vi.fn()}
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Revoke connection" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "secret-bearing verification data",
    );
    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
  });
});
