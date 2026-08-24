import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { AssistantEndpointDeleteDialog } from "./assistant-endpoint-delete-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => {
  class MockApiError extends Error {
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
  endpointId: "00000000-0000-4000-8000-000000000001",
} as const;

function renderDialog(onComplete = vi.fn()) {
  const onOpenChange = vi.fn();
  const rendered = render(
    <AssistantEndpointDeleteDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="action-endpoint-delete"
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

describe("AssistantEndpointDeleteDialog", () => {
  it("confirms every time and reports only after evidence 404", async () => {
    mockPost.mockResolvedValue({
      resource: { endpointId: PARAMS.endpointId },
      replayed: false,
    });
    mockGet.mockRejectedValue(notFoundError());
    const { onComplete } = renderDialog();

    expect(
      screen.getByText(/Confirm every time/i),
    ).toBeInTheDocument();
    const submit = screen.getByRole("button", { name: "Delete endpoint" });
    fireEvent.click(submit);
    fireEvent.click(submit);

    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/endpoints/delete",
      {
        actionRequestId: "action-endpoint-delete",
        endpointId: PARAMS.endpointId,
      },
    );
    expect(mockGet).toHaveBeenCalledWith(
      `/endpoints/${PARAMS.endpointId}/authorization`,
    );
    expect(
      await screen.findByText("Endpoint absence verified."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Report deletion" }));
    expect(onComplete).toHaveBeenCalledWith(PARAMS.endpointId);
  });

  it("does not finish when the projection still returns the endpoint", async () => {
    mockPost.mockResolvedValue({
      resource: { endpointId: PARAMS.endpointId },
      replayed: false,
    });
    mockGet.mockResolvedValue({
      id: PARAMS.endpointId,
      auto_connected: false,
      catalog_service_id: null,
      updated_at: "2026-08-20T10:00:00Z",
    });
    renderDialog();
    fireEvent.click(screen.getByRole("button", { name: "Delete endpoint" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Report deletion" })).toBeDisabled();
  });
});
