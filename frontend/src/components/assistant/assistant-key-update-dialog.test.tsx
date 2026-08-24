import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AssistantKeyUpdateDialog,
  type AssistantKeyUpdateParams,
} from "./assistant-key-update-dialog";

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

const PARAMS: AssistantKeyUpdateParams = {
  keyId: "00000000-0000-4000-8000-000000000001",
  name: "renamed-agent",
};

function evidence(overrides: Record<string, unknown> = {}) {
  return {
    id: PARAMS.keyId,
    name: "coding-agent",
    platform: "codex",
    is_active: true,
    state_version: 1,
    allowed_service_ids: ["service-alpha"],
    allow_all_services: false,
    ...overrides,
  };
}

function renderDialog(
  params: AssistantKeyUpdateParams = PARAMS,
  onComplete = vi.fn(),
) {
  const onOpenChange = vi.fn();
  const rendered = render(
    <AssistantKeyUpdateDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="action-update-alpha"
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

describe("AssistantKeyUpdateDialog", () => {
  it("fences double submission and reports only after evidence verification", async () => {
    mockGet
      .mockResolvedValueOnce(evidence())
      .mockResolvedValueOnce(evidence({ name: "renamed-agent", state_version: 2 }));
    mockPost.mockResolvedValue({
      resource: { keyId: PARAMS.keyId },
      replayed: false,
    });
    const { onComplete } = renderDialog();

    const update = screen.getByRole("button", { name: "Update key" });
    fireEvent.click(update);
    fireEvent.click(update);

    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/keys/update", {
      actionRequestId: "action-update-alpha",
      keyId: PARAMS.keyId,
      name: "renamed-agent",
      platform: undefined,
      description: undefined,
      expectedStateVersion: 1,
    });
    expect(
      await screen.findByText("Exact key metadata verified."),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(PARAMS.keyId);
  });

  it("rejects a secret-shaped name before mutation", async () => {
    renderDialog({
      keyId: PARAMS.keyId,
      name: "Bearer sk-live-abc123def456",
    });
    await userEvent.click(screen.getByRole("button", { name: "Update key" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(mockPost).not.toHaveBeenCalled();
  });

  it("keeps the report blocked when evidence does not match", async () => {
    mockGet
      .mockResolvedValueOnce(evidence())
      .mockResolvedValueOnce(evidence({ name: "other-name", state_version: 2 }));
    mockPost.mockResolvedValue({
      resource: { keyId: PARAMS.keyId },
      replayed: false,
    });
    const { onComplete } = renderDialog();
    await userEvent.click(screen.getByRole("button", { name: "Update key" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
    expect(onComplete).not.toHaveBeenCalled();
  });
});
