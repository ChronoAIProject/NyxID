import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { AssistantExternalKeyDeleteDialog } from "./assistant-external-key-delete-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => {
  class MockApiError extends Error {
    readonly status: number;
    readonly errorCode: number;
    readonly errorResponse: unknown;
    constructor(
      status: number,
      response: { error: string; error_code: number; message: string; details?: unknown },
    ) {
      super(response.message);
      this.status = status;
      this.errorCode = response.error_code;
      this.errorResponse = response;
    }
  }
  return {
    api: { get: mockGet, post: mockPost },
    ApiError: MockApiError,
  };
});

function notFoundError() {
  return new ApiError(404, {
    error: "not_found",
    error_code: 1003,
    message: "not found",
  });
}

const PARAMS = {
  externalKeyId: "00000000-0000-4000-8000-0000000000bb",
} as const;

const CASCADE_RESPONSE = {
  error: "grant_cascade_confirmation_required",
  error_code: 11500,
  message: "Grant cascade confirmation required",
  details: {
    provider_slug: "github",
    provider_name: "GitHub",
    revokes_grant: true,
    siblings: [
      {
        user_service_id: "svc-sibling",
        name: "Sibling",
        slug: "svc-sibling",
      },
    ],
    unaffected_other_app: [],
    token_scope_available: true,
  },
};

function renderDialog(onComplete = vi.fn()) {
  const onOpenChange = vi.fn();
  const rendered = render(
    <AssistantExternalKeyDeleteDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="action-ext-delete"
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

describe("AssistantExternalKeyDeleteDialog", () => {
  it("confirms every time and reports only after evidence 404", async () => {
    mockPost.mockResolvedValue({
      resource: { externalKeyId: PARAMS.externalKeyId },
      replayed: false,
    });
    mockGet.mockRejectedValue(notFoundError());
    const { onComplete } = renderDialog();

    expect(screen.getByText(/Confirm every time/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Delete credential" }));
    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockGet).toHaveBeenCalledWith(
      `/api-keys/external/${PARAMS.externalKeyId}/authorization`,
    );
    expect(
      await screen.findByText("External credential absence verified."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Report deletion" }));
    expect(onComplete).toHaveBeenCalledWith(PARAMS.externalKeyId);
  });

  it("retries with cascadeGrant after the 11500 confirmation contract", async () => {
    mockPost
      .mockRejectedValueOnce(new ApiError(409, CASCADE_RESPONSE))
      .mockResolvedValueOnce({
        resource: { externalKeyId: PARAMS.externalKeyId },
        replayed: false,
      });
    mockGet.mockRejectedValue(notFoundError());
    renderDialog();

    fireEvent.click(screen.getByRole("button", { name: "Delete credential" }));
    expect(
      await screen.findByText("Disconnect GitHub everywhere?"),
    ).toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", {
        name: "Disconnect GitHub everywhere (2 services)",
      }),
    );
    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(2));
    expect(mockPost).toHaveBeenLastCalledWith(
      "/assistant/actions/endpoints/external-key-delete",
      {
        actionRequestId: "action-ext-delete",
        externalKeyId: PARAMS.externalKeyId,
        cascadeGrant: true,
      },
    );
    expect(
      await screen.findByText("External credential absence verified."),
    ).toBeInTheDocument();
  });
});
