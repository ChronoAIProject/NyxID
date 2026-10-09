import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import type { KeyInfo } from "@/types/keys";
import { ConnectionDeleteAction } from "./connection-delete-action";

const { mutate, errorToast } = vi.hoisted(() => ({
  mutate: vi.fn(),
  errorToast: vi.fn(),
}));
vi.mock("@/hooks/use-keys", () => ({
  useDeleteKey: () => ({ mutate, isPending: false }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: errorToast } }));
const connection = {
  id: "work-id",
  label: "Work",
  slug: "openai-work",
} as KeyInfo;
afterEach(cleanup);
beforeEach(() => vi.clearAllMocks());

it("requires confirmation and deletes only the selected connection without navigation", async () => {
  const user = userEvent.setup();
  render(<ConnectionDeleteAction connection={connection} />);
  await user.click(
    screen.getByRole("button", {
      name: "Delete connection Work (openai-work)",
    }),
  );
  expect(mutate).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(mutate).not.toHaveBeenCalled();
  await user.click(
    screen.getByRole("button", {
      name: "Delete connection Work (openai-work)",
    }),
  );
  await user.click(screen.getByRole("button", { name: "Delete connection" }));
  expect(mutate).toHaveBeenCalledWith("work-id", expect.any(Object));
  act(() =>
    mutate.mock.calls[0]?.[1].onSuccess({ upstream_revocation_scheduled: false }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("requires a separate choice before removing a shared OAuth authorization", async () => {
  mutate.mockImplementation((_input, callbacks) =>
    callbacks.onError(
      new ApiError(409, {
        error: "grant_cascade_confirmation_required",
        error_code: 11500,
        message: "Confirmation required",
        details: {
          provider_slug: "google",
          provider_name: "Google",
          revokes_grant: true,
          siblings: [
            { user_service_id: "other", name: "Other", slug: "other" },
          ],
          unaffected_other_app: [],
          token_scope_available: true,
        },
      }),
    ),
  );
  const user = userEvent.setup();
  render(<ConnectionDeleteAction connection={connection} />);
  await user.click(
    screen.getByRole("button", {
      name: "Delete connection Work (openai-work)",
    }),
  );
  await user.click(screen.getByRole("button", { name: "Delete connection" }));
  expect(mutate).toHaveBeenCalledTimes(1);
  mutate.mockImplementation(() => {});
  await user.click(
    screen.getByRole("button", { name: "Remove only this service" }),
  );
  expect(mutate).toHaveBeenLastCalledWith(
    { keyId: "work-id", grantScope: "token" },
    expect.any(Object),
  );
});

it("keeps confirmation open after a failed delete", async () => {
  mutate.mockImplementation((_input, callbacks) =>
    callbacks.onError(new Error("offline")),
  );
  const user = userEvent.setup();
  render(<ConnectionDeleteAction connection={connection} />);
  await user.click(
    screen.getByRole("button", {
      name: "Delete connection Work (openai-work)",
    }),
  );
  await user.click(screen.getByRole("button", { name: "Delete connection" }));
  expect(errorToast).toHaveBeenCalledWith("Failed to delete connection");
  expect(screen.getByRole("dialog")).toBeInTheDocument();
});

it("prevents deletion while reordering", () => {
  render(<ConnectionDeleteAction connection={connection} disabled />);
  expect(
    screen.getByRole("button", {
      name: "Delete connection Work (openai-work)",
    }),
  ).toBeDisabled();
});
