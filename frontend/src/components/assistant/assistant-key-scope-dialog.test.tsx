import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AssistantKeyScopeDialog,
  type AssistantKeyScopeParams,
} from "./assistant-key-scope-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error {},
}));

const PARAMS: AssistantKeyScopeParams = {
  keyId: "00000000-0000-4000-8000-000000000001",
  addServiceIds: ["00000000-0000-4000-8000-0000000000aa"],
};

function evidence(overrides: Record<string, unknown> = {}) {
  return {
    id: PARAMS.keyId,
    is_active: true,
    allowed_service_ids: ["00000000-0000-4000-8000-0000000000bb"],
    allow_all_services: false,
    state_version: 1,
    ...overrides,
  };
}

function renderDialog(
  params: AssistantKeyScopeParams = PARAMS,
  onComplete = vi.fn(),
) {
  const onOpenChange = vi.fn();
  const rendered = render(
    <AssistantKeyScopeDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="action-scope-alpha"
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

describe("AssistantKeyScopeDialog", () => {
  it("fences double submission and verifies the added service ids", async () => {
    mockGet
      .mockResolvedValueOnce(evidence())
      .mockResolvedValueOnce(
        evidence({
          allowed_service_ids: [
            "00000000-0000-4000-8000-0000000000bb",
            PARAMS.addServiceIds[0],
          ],
          state_version: 2,
        }),
      );
    mockPost.mockResolvedValue({
      resource: { keyId: PARAMS.keyId },
      replayed: false,
    });
    const { onComplete } = renderDialog();

    const extend = screen.getByRole("button", { name: "Extend scope" });
    fireEvent.click(extend);
    fireEvent.click(extend);

    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/keys/extend-scope",
      {
        actionRequestId: "action-scope-alpha",
        keyId: PARAMS.keyId,
        addServiceIds: PARAMS.addServiceIds,
        expectedStateVersion: 1,
      },
    );
    expect(
      await screen.findByText("Exact widened service set verified."),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(PARAMS.keyId);
  });

  it("rejects evidence that sets allow_all_services", async () => {
    mockGet
      .mockResolvedValueOnce(evidence())
      .mockResolvedValueOnce(
        evidence({
          allow_all_services: true,
          allowed_service_ids: PARAMS.addServiceIds,
          state_version: 2,
        }),
      );
    mockPost.mockResolvedValue({
      resource: { keyId: PARAMS.keyId },
      replayed: false,
    });
    const { onComplete } = renderDialog();
    await userEvent.click(screen.getByRole("button", { name: "Extend scope" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
    expect(onComplete).not.toHaveBeenCalled();
  });

  it("does not offer a remember path", () => {
    renderDialog();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    expect(screen.getByText(/never remembered/i)).toBeInTheDocument();
  });
});
