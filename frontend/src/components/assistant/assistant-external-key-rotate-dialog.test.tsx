import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantExternalKeyRotateDialog } from "./assistant-external-key-rotate-dialog";

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

const PARAMS = {
  externalKeyId: "00000000-0000-4000-8000-0000000000aa",
} as const;

function evidence(overrides: Record<string, unknown> = {}) {
  return {
    id: PARAMS.externalKeyId,
    credential_type: "api_key",
    status: "active",
    expires_at: null,
    last_used_at: null,
    updated_at: "2026-08-20T10:00:01Z",
    ...overrides,
  };
}

function renderDialog(onComplete = vi.fn()) {
  const onOpenChange = vi.fn();
  const rendered = render(
    <AssistantExternalKeyRotateDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="action-ext-rotate"
      params={PARAMS}
      onComplete={onComplete}
    />,
  );
  return { ...rendered, onComplete, onOpenChange };
}

beforeEach(() => {
  mockGet.mockReset();
  mockPost.mockReset();
});

describe("AssistantExternalKeyRotateDialog", () => {
  it("requires a replacement secret and reports only after pinned-enum evidence read-back", async () => {
    mockPost.mockResolvedValue({
      resource: { externalKeyId: PARAMS.externalKeyId },
      replayed: false,
    });
    mockGet.mockResolvedValue(evidence());
    const { onComplete } = renderDialog();

    const submit = screen.getByRole("button", { name: "Rotate credential" });
    expect(submit).toBeDisabled();
    await userEvent.type(
      screen.getByLabelText("Replacement credential"),
      "sk-replacement-secret",
    );
    expect(submit).toBeEnabled();
    fireEvent.click(submit);
    fireEvent.click(submit);

    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/endpoints/external-key-rotate",
      {
        actionRequestId: "action-ext-rotate",
        externalKeyId: PARAMS.externalKeyId,
        credential: "sk-replacement-secret",
      },
    );
    expect(mockGet).toHaveBeenCalledWith(
      `/api-keys/external/${PARAMS.externalKeyId}/authorization`,
    );
    expect(
      await screen.findByText("External credential rotation verified."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Report rotation" }));
    expect(onComplete).toHaveBeenCalledWith(PARAMS.externalKeyId);
    expect(onComplete).not.toHaveBeenCalledWith(
      expect.stringContaining("replacement"),
    );
  });

  it("refuses to finish when evidence carries error_message", async () => {
    mockPost.mockResolvedValue({
      resource: { externalKeyId: PARAMS.externalKeyId },
      replayed: false,
    });
    mockGet.mockResolvedValue({
      ...evidence(),
      error_message: "Bearer nyxid_ag_abcdefghijklmnopqrst",
    });
    renderDialog();
    await userEvent.type(
      screen.getByLabelText("Replacement credential"),
      "sk-replacement-secret",
    );
    fireEvent.click(screen.getByRole("button", { name: "Rotate credential" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Report rotation" })).toBeDisabled();
  });
});
