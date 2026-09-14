import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { HandoffCard } from "./handoff-card";
import { handoffFormSchema } from "@/schemas/app-connect-links";

const mocks = vi.hoisted(() => ({ update: vi.fn() }));
vi.mock("@/hooks/use-app-connect-links", () => ({
  useUpdateAppHandoff: () => ({
    mutateAsync: mocks.update,
    isPending: false,
    error: null,
  }),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mocks.update.mockImplementation(async (input) => input);
});
it("only saves edited handoff text and restores the clean form after saving", async () => {
  render(<HandoffCard clientId="app" blurb="Connect to continue" />);
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  fireEvent.change(screen.getByLabelText("Handoff text"), {
    target: { value: "Choose your accounts" },
  });
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(mocks.update).toHaveBeenCalledExactlyOnceWith({
      handoff_blurb: "Choose your accounts",
    }),
  );
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled(),
  );
});
it("bounds text to 160 characters and permits clearing", () => {
  expect(
    handoffFormSchema.safeParse({ handoff_blurb: "x".repeat(161) }).success,
  ).toBe(false);
  expect(handoffFormSchema.parse({ handoff_blurb: "   " }).handoff_blurb).toBe(
    "",
  );
});
