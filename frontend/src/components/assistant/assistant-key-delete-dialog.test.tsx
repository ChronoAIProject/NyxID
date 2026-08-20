import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import {
  AssistantKeyDeleteDialog,
  type AssistantKeyDeleteParams,
} from "./assistant-key-delete-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error {
    readonly status: number;
    constructor(status: number, response?: { message?: string }) {
      super(response?.message ?? `HTTP ${String(status)}`);
      this.status = status;
    }
  },
}));

const PARAMS: AssistantKeyDeleteParams = {
  keyId: "00000000-0000-4000-8000-000000000001",
};

function evidence(overrides: Record<string, unknown> = {}) {
  return {
    id: PARAMS.keyId,
    name: "coding-agent",
    is_active: true,
    state_version: 1,
    ...overrides,
  };
}

function renderDialog(
  params: AssistantKeyDeleteParams = PARAMS,
  onComplete = vi.fn(),
) {
  const onOpenChange = vi.fn();
  const rendered = render(
    <AssistantKeyDeleteDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="action-delete-alpha"
      params={params}
      onComplete={onComplete}
    />,
  );
  return { ...rendered, onComplete, onOpenChange };
}

beforeEach(() => {
  mockGet.mockReset();
  mockPost.mockReset();
});

describe("AssistantKeyDeleteDialog", () => {
  it("confirms every time and reports only after a 404 evidence read", async () => {
    mockGet
      .mockResolvedValueOnce(evidence())
      .mockRejectedValueOnce(
        new ApiError(404, {
          error: "not_found",
          error_code: 1003,
          message: "API key not found",
        }),
      );
    mockPost.mockResolvedValue({
      resource: { keyId: PARAMS.keyId },
      replayed: false,
    });
    const { onComplete } = renderDialog();

    expect(
      screen.getByText(/confirm this exact key every time/i),
    ).toBeInTheDocument();
    const del = screen.getByRole("button", { name: "Delete key" });
    fireEvent.click(del);
    fireEvent.click(del);

    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/keys/delete", {
      actionRequestId: "action-delete-alpha",
      keyId: PARAMS.keyId,
      expectedStateVersion: 1,
    });
    expect(mockGet).toHaveBeenLastCalledWith(
      `/api-keys/${PARAMS.keyId}/authorization`,
    );
    expect(
      await screen.findByText("Authorization evidence is absent."),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(PARAMS.keyId);
  });

  it("keeps the report blocked when the projection still returns the key", async () => {
    mockGet.mockResolvedValue(evidence({ state_version: 2 }));
    mockPost.mockResolvedValue({
      resource: { keyId: PARAMS.keyId },
      replayed: false,
    });
    const { onComplete } = renderDialog();
    await userEvent.click(screen.getByRole("button", { name: "Delete key" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "still returned authorization evidence",
    );
    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
    expect(onComplete).not.toHaveBeenCalled();
  });

  it("rejects malformed key identities before mutation", async () => {
    renderDialog({ keyId: "invalid/key" });
    await userEvent.click(screen.getByRole("button", { name: "Delete key" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(mockPost).not.toHaveBeenCalled();
  });
});
