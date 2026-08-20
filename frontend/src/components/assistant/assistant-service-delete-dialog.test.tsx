import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { AssistantServiceDeleteDialog } from "./assistant-service-delete-dialog";

const { mockPost } = vi.hoisted(() => ({
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => {
  class ApiError extends Error {
    readonly status: number;
    readonly errorCode: number;
    readonly errorResponse: { error: string; error_code: number; message: string; details?: unknown };
    constructor(
      status: number,
      response: {
        error: string;
        error_code: number;
        message: string;
        details?: unknown;
      },
    ) {
      super(response.message);
      this.status = status;
      this.errorCode = response.error_code;
      this.errorResponse = response;
    }
  }
  return {
    api: { post: mockPost },
    ApiError,
  };
});

const SERVICE_ID = "11111111-1111-4111-8111-111111111111";

function renderDialog(onComplete = vi.fn(), onOpenChange = vi.fn()) {
  return render(
    <AssistantServiceDeleteDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="act-delete"
      params={{ userServiceId: SERVICE_ID }}
      onComplete={onComplete}
    />,
  );
}

beforeEach(() => {
  mockPost.mockReset();
});

describe("AssistantServiceDeleteDialog", () => {
  it("requires an explicit confirm before every delete, including retries", async () => {
    mockPost.mockResolvedValue({
      resource: { userServiceId: SERVICE_ID },
      replayed: false,
    });
    const onComplete = vi.fn();
    const { rerender } = renderDialog(onComplete);

    expect(mockPost).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole("button", { name: "I understand, continue" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Delete service" }),
    );
    expect(mockPost).toHaveBeenCalledTimes(1);
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/services/delete",
      {
        actionRequestId: "act-delete",
        userServiceId: SERVICE_ID,
      },
    );
    expect(onComplete).toHaveBeenCalledWith(SERVICE_ID);

    rerender(
      <AssistantServiceDeleteDialog
        key="second-open"
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-delete-again"
        params={{ userServiceId: SERVICE_ID }}
        onComplete={onComplete}
      />,
    );
    expect(
      screen.getByRole("button", { name: "I understand, continue" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Delete service" }),
    ).not.toBeInTheDocument();
  });

  it("honours grant cascade confirmation before deleting", async () => {
    mockPost.mockRejectedValueOnce(
      new ApiError(409, {
        error: "grant_cascade_confirmation_required",
        error_code: 11_500,
        message: "Grant cascade confirmation required",
        details: {
          provider_slug: "github",
          provider_name: "GitHub",
          revokes_grant: true,
          siblings: [
            {
              user_service_id: "sibling",
              name: "other",
              slug: "other",
            },
          ],
          token_scope_available: true,
        },
      }),
    );
    mockPost.mockResolvedValueOnce({
      resource: { userServiceId: SERVICE_ID },
      replayed: false,
    });
    const onComplete = vi.fn();
    renderDialog(onComplete);

    await userEvent.click(
      screen.getByRole("button", { name: "I understand, continue" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Delete service" }),
    );
    expect(
      await screen.findByText(/revoke the shared grant/i),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Delete and revoke grant" }),
    );
    expect(mockPost).toHaveBeenLastCalledWith(
      "/assistant/actions/services/delete",
      {
        actionRequestId: "act-delete",
        userServiceId: SERVICE_ID,
        cascadeGrant: true,
      },
    );
    expect(onComplete).toHaveBeenCalledWith(SERVICE_ID);
  });
});
