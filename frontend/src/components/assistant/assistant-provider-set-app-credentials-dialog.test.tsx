import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantProviderSetAppCredentialsDialog } from "./assistant-provider-set-app-credentials-dialog";

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

const PROVIDER_ID = "33333333-3333-4333-8333-333333333333";

beforeEach(() => {
  mockGet.mockReset();
  mockPost.mockReset();
});

describe("AssistantProviderSetAppCredentialsDialog", () => {
  it("accepts a password only in-browser, clears it, and reports after read-back", async () => {
    mockGet
      .mockResolvedValueOnce({
        provider_id: PROVIDER_ID,
        has_credentials: false,
        state_version: 0,
        updated_at: null,
      })
      .mockResolvedValueOnce({
        provider_id: PROVIDER_ID,
        has_credentials: true,
        state_version: 1,
        updated_at: "2026-08-25T00:00:01Z",
      });
    mockPost.mockResolvedValue({
      resource: { providerId: PROVIDER_ID },
      replayed: false,
      oneTimeMaterial: "unavailable",
    });
    const onComplete = vi.fn();
    render(
      <AssistantProviderSetAppCredentialsDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-provider-credentials"
        params={{ providerId: PROVIDER_ID }}
        onComplete={onComplete}
      />,
    );
    const secretInput = screen.getByLabelText("Client secret");
    expect(secretInput).toHaveAttribute("type", "password");
    await userEvent.type(screen.getByLabelText("Client ID"), "browser-client-id");
    await userEvent.type(secretInput, "browser-client-secret");
    await userEvent.click(
      screen.getByRole("button", { name: "Save credentials" }),
    );
    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/providers/set-app-credentials",
      {
        actionRequestId: "act-provider-credentials",
        providerId: PROVIDER_ID,
        clientId: "browser-client-id",
        clientSecret: "browser-client-secret",
        expectedStateVersion: 0,
      },
    );
    expect(screen.queryByDisplayValue("browser-client-secret")).toBeNull();
    expect(screen.queryByText("browser-client-secret")).not.toBeInTheDocument();
    expect(await screen.findByText("Credential evidence verified.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(PROVIDER_ID);
  });

  it("rejects secret-shaped credential evidence and never reports it", async () => {
    mockGet
      .mockResolvedValueOnce({
        provider_id: PROVIDER_ID,
        has_credentials: false,
        state_version: 0,
        updated_at: null,
      })
      .mockResolvedValueOnce({
        provider_id: PROVIDER_ID,
        has_credentials: true,
        state_version: 1,
        updated_at: "2026-08-25T00:00:01Z",
        client_secret: "must-not-return",
      });
    mockPost.mockResolvedValue({
      resource: { providerId: PROVIDER_ID },
      replayed: false,
    });
    render(
      <AssistantProviderSetAppCredentialsDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-provider-credentials-poison"
        params={{ providerId: PROVIDER_ID }}
        onComplete={vi.fn()}
      />,
    );
    await userEvent.type(screen.getByLabelText("Client ID"), "browser-client-id");
    await userEvent.type(screen.getByLabelText("Client secret"), "browser-secret");
    await userEvent.click(
      screen.getByRole("button", { name: "Save credentials" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "secret-bearing verification data",
    );
    expect(screen.queryByText("must-not-return")).not.toBeInTheDocument();
  });
});
