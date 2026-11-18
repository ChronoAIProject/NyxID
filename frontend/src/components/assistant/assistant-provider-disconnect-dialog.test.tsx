import { invalidateAccountEvidence, settleAccountEvidence } from "@/lib/assistant/identity";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantProviderDisconnectDialog } from "./assistant-provider-disconnect-dialog";

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

const PROVIDER_ID = "22222222-2222-4222-8222-222222222222";

beforeEach(() => {
  mockGet.mockReset();
  mockPost.mockReset();
});

describe("AssistantProviderDisconnectDialog", () => {
  it("posts explicit confirmation and verifies the revoked token state", async () => {
    mockGet
      .mockResolvedValueOnce({
        provider_id: PROVIDER_ID,
        status: "active",
        state_version: 1,
        updated_at: "2026-08-25T00:00:00Z",
      })
      .mockResolvedValueOnce({
        provider_id: PROVIDER_ID,
        status: "revoked",
        state_version: 2,
        updated_at: "2026-08-25T00:00:01Z",
      });
    mockPost.mockResolvedValue({
      resource: { providerId: PROVIDER_ID },
      replayed: false,
    });
    const onComplete = vi.fn();
    render(
      <AssistantProviderDisconnectDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-provider-disconnect"
        params={{ providerId: PROVIDER_ID }}
        onComplete={onComplete}
      />,
    );
    const disconnectButton = screen.getByRole("button", {
      name: "Disconnect provider",
    });
    expect(disconnectButton).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox"));
    expect(disconnectButton).toBeEnabled();
    await userEvent.click(disconnectButton);
    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/providers/provider-disconnect",
      {
        actionRequestId: "act-provider-disconnect",
        providerId: PROVIDER_ID,
        expectedStateVersion: 1,
        confirmed: true,
      },
    );
    expect(await screen.findByText("Disconnect evidence verified.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(PROVIDER_ID);
  });

  it("rejects a secret-bearing effect response", async () => {
    mockGet.mockResolvedValue({
      provider_id: PROVIDER_ID,
      status: "active",
      state_version: 1,
      updated_at: "2026-08-25T00:00:00Z",
    });
    mockPost.mockResolvedValue({
      resource: { providerId: PROVIDER_ID },
      replayed: false,
      accessToken: "must-not-return",
    });
    render(
      <AssistantProviderDisconnectDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-provider-poison"
        params={{ providerId: PROVIDER_ID }}
        onComplete={vi.fn()}
      />,
    );
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(
      screen.getByRole("button", { name: "Disconnect provider" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "secret-bearing verification data",
    );
    expect(mockGet).toHaveBeenCalledTimes(1);
  });
});

it("silently stops a confirmed effect when evidence arrives during read-back", async () => {
  let finish!: (value: unknown) => void;
  mockGet.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  render(<AssistantProviderDisconnectDialog open onOpenChange={vi.fn()} actionRequestId="fixture-action"
    params={{ providerId: PROVIDER_ID }} onComplete={vi.fn()} />);
  await userEvent.click(screen.getByRole("checkbox"));
  await userEvent.click(screen.getByRole("button", { name: "Disconnect provider" }));
  await waitFor(() => expect(finish).toBeDefined());
  invalidateAccountEvidence(); settleAccountEvidence();
  finish({ provider_id: PROVIDER_ID, status: "active", state_version: 1, updated_at: "2026-08-25T00:00:00Z" });
  await waitFor(() => expect(screen.getByRole("button", { name: "Disconnect provider" })).toBeEnabled());
  expect(mockPost).not.toHaveBeenCalled();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.queryByText("Account changed")).not.toBeInTheDocument();
});
