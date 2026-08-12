import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const trigger = {
  id: "80c7e6d9-41d5-48c3-bfd7-2bf9c92fa288",
  user_id: "1ca34962-7698-46a0-85d0-c85445becd72",
  label: "Repository activity",
  user_service_id: null,
  status: "active" as const,
  verification: { mode: "token" as const, location: "bearer" as const },
  delivery: { type: "notification" as const },
  inbound_url:
    "https://api.example.com/api/v1/webhooks/triggers/80c7e6d9-41d5-48c3-bfd7-2bf9c92fa288",
  created_at: "2026-08-06T09:30:00.123+00:00",
  updated_at: "2026-08-06T09:30:00.123+00:00",
};

const mocks = vi.hoisted(() => ({
  create: vi.fn(),
  remove: vi.fn(),
  rotate: vi.fn(),
  update: vi.fn(),
  useTriggers: vi.fn(),
}));

vi.mock("@/hooks/use-triggers", () => ({
  useCreateTrigger: () => ({ mutateAsync: mocks.create, isPending: false }),
  useDeleteTrigger: () => ({ mutateAsync: mocks.remove, isPending: false }),
  useRotateTriggerSecret: () => ({ mutateAsync: mocks.rotate, isPending: false }),
  useTriggers: mocks.useTriggers,
  useUpdateTrigger: () => ({ mutateAsync: mocks.update, isPending: false }),
}));

import { TriggersPage } from "./triggers";

beforeEach(() => {
  vi.clearAllMocks();
  mocks.useTriggers.mockReturnValue({
    data: { triggers: [] },
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  });
  mocks.create.mockResolvedValue({
    trigger,
    secret: "nyx_trg_once",
    delivery_signing_secret: null,
  });
  mocks.update.mockResolvedValue({
    trigger: { ...trigger, status: "disabled" },
    delivery_signing_secret: null,
  });
});

describe("TriggersPage", () => {
  it("creates a trigger with exact tagged unions and reveals backend values", async () => {
    const user = userEvent.setup();
    render(<TriggersPage />);

    expect(screen.getByText("No triggers yet")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Create Trigger" }));
    await user.type(screen.getByLabelText("Label"), "Repository activity");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() =>
      expect(mocks.create).toHaveBeenCalledWith({
        label: "Repository activity",
        verification: { mode: "token", location: "bearer" },
        delivery: { type: "notification" },
      }),
    );
    expect(screen.getByText(trigger.inbound_url)).toBeInTheDocument();
    expect(screen.getByText("nyx_trg_once")).toBeInTheDocument();
  });

  it("disables a populated trigger from the row action menu", async () => {
    mocks.useTriggers.mockReturnValue({
      data: { triggers: [trigger] },
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    });
    const user = userEvent.setup();
    render(<TriggersPage />);

    await user.click(
      screen.getAllByRole("button", {
        name: "More actions for Repository activity",
      })[0]!,
    );
    await user.click(screen.getByRole("menuitem", { name: "Disable" }));

    await waitFor(() =>
      expect(mocks.update).toHaveBeenCalledWith({
        id: trigger.id,
        data: { status: "disabled" },
      }),
    );
  });
});
