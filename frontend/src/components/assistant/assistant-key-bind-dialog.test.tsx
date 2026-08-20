import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AssistantKeyBindDialog,
  type AssistantKeyBindParams,
} from "./assistant-key-bind-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error {},
}));

const PARAMS: AssistantKeyBindParams = {
  keyId: "00000000-0000-4000-8000-000000000001",
  userServiceId: "00000000-0000-4000-8000-0000000000aa",
  externalKeyId: "00000000-0000-4000-8000-0000000000cc",
};
const BINDING_ID = "00000000-0000-4000-8000-0000000000bb";

function bindingEvidence(overrides: Record<string, unknown> = {}) {
  return {
    id: BINDING_ID,
    api_key_id: PARAMS.keyId,
    user_service_id: PARAMS.userServiceId,
    user_api_key_id: PARAMS.externalKeyId,
    created_at: "2026-08-11T08:00:00Z",
    updated_at: "2026-08-11T08:00:00Z",
    ...overrides,
  };
}

function renderDialog(
  params: AssistantKeyBindParams = PARAMS,
  onComplete = vi.fn(),
) {
  const onOpenChange = vi.fn();
  const rendered = render(
    <AssistantKeyBindDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="action-bind-alpha"
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

describe("AssistantKeyBindDialog", () => {
  it("fences double submission and reports only verified binding evidence", async () => {
    mockPost.mockResolvedValue({
      resource: { keyId: PARAMS.keyId },
      bindingId: BINDING_ID,
      replayed: false,
    });
    mockGet.mockResolvedValue(bindingEvidence());
    const { onComplete } = renderDialog();

    const bind = screen.getByRole("button", { name: "Bind credential" });
    fireEvent.click(bind);
    fireEvent.click(bind);

    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/keys/bind-credential",
      {
        actionRequestId: "action-bind-alpha",
        keyId: PARAMS.keyId,
        userServiceId: PARAMS.userServiceId,
        externalKeyId: PARAMS.externalKeyId,
      },
    );
    expect(mockGet).toHaveBeenCalledWith(
      `/api-keys/${PARAMS.keyId}/bindings/${BINDING_ID}/authorization`,
    );
    expect(
      await screen.findByText("Exact binding evidence verified."),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(PARAMS.keyId);
  });

  it("rejects evidence that still carries labels", async () => {
    mockPost.mockResolvedValue({
      resource: { keyId: PARAMS.keyId },
      bindingId: BINDING_ID,
      replayed: false,
    });
    mockGet.mockResolvedValue(
      bindingEvidence({ service_label: "Bearer Bot" }),
    );
    const { onComplete } = renderDialog();
    await userEvent.click(
      screen.getByRole("button", { name: "Bind credential" }),
    );
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
