import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mockNavigate = vi.fn();

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

const mockMutateAsync = vi.fn();

vi.mock("@/hooks/use-api-keys", () => ({
  useRotateApiKey: () => ({
    mutateAsync: mockMutateAsync,
    isPending: false,
  }),
}));

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}));

import { RotateKeyDialog } from "./rotate-key-dialog";

describe("RotateKeyDialog", () => {
  const OLD_KEY_ID = "old-key-id";
  const NEW_KEY_ID = "new-key-id";

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("navigates to the new key detail page after rotation and dismissal", async () => {
    const user = userEvent.setup();
    mockMutateAsync.mockResolvedValue({
      id: NEW_KEY_ID,
      full_key: "nyxid_ag_newkey123",
      name: "Test Key",
      key_prefix: "nyxid_ag_ne",
      scopes: "proxy",
      created_at: "2026-04-14T00:00:00Z",
    });

    render(
      <RotateKeyDialog
        open
        onOpenChange={vi.fn()}
        keyId={OLD_KEY_ID}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Rotate Key" }));

    await waitFor(() => {
      expect(screen.getByText("New API Key")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "Done" }));

    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/keys/api-key/$keyId",
      params: { keyId: NEW_KEY_ID },
    });
  });

  it("closes and selects an adopted assistant successor without a copy-secret screen", async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    mockMutateAsync.mockResolvedValue({ id: NEW_KEY_ID, full_key: "" });
    render(<RotateKeyDialog open onOpenChange={onOpenChange} keyId={OLD_KEY_ID} />);

    await user.click(screen.getByRole("button", { name: "Rotate Key" }));

    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/keys/api-key/$keyId",
      params: { keyId: NEW_KEY_ID },
    });
    expect(screen.queryByText("New API Key")).not.toBeInTheDocument();
  });

  it("does not navigate when dialog is cancelled before rotation", async () => {
    const user = userEvent.setup();

    render(
      <RotateKeyDialog
        open
        onOpenChange={vi.fn()}
        keyId={OLD_KEY_ID}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(mockNavigate).not.toHaveBeenCalled();
  });
});
