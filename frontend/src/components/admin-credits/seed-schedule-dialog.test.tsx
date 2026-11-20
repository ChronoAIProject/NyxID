import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { CreditSchedule } from "@/schemas/billing-credits";
import { SeedScheduleDialog } from "./seed-schedule-dialog";

const mock = vi.hoisted(() => ({
  mutate: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}));
vi.mock("@/hooks/use-billing-credits", () => ({
  useSeedCreditSchedule: () => ({ mutateAsync: mock.mutate, isPending: false }),
}));
vi.mock("sonner", () => ({
  toast: { success: mock.success, error: mock.error },
}));
const schedule = {
  id: "schedule-1",
  amount: "200",
  amount_credits: 200,
  updated_at: "2026-11-05T00:00:00Z",
  current_period: { amount: "100.123456789012", start: "2026-11-01T00:00:00Z" },
} as CreditSchedule;
beforeEach(() => vi.clearAllMocks());

it("explains the current-period amount and issues only after confirmation", async () => {
  mock.mutate.mockResolvedValue({ created: 2, existing: 3, pending: 0 });
  const onClose = vi.fn();
  render(<SeedScheduleDialog schedule={schedule} onClose={onClose} />);
  expect(screen.getByText(/100.123456789012 credits/)).toBeInTheDocument();
  expect(screen.getByText(/spent, expired or reclaimed/)).toBeInTheDocument();
  expect(mock.mutate).not.toHaveBeenCalled();
  await userEvent.click(
    screen.getByRole("button", { name: "Grant missing credits" }),
  );
  await waitFor(() =>
    expect(mock.mutate).toHaveBeenCalledWith({
      id: "schedule-1",
      updatedAt: "2026-11-05T00:00:00Z",
      periodStart: "2026-11-01T00:00:00Z",
    }),
  );
  expect(onClose).toHaveBeenCalledOnce();
  expect(mock.success).toHaveBeenCalledWith(
    expect.stringContaining("2 users; 3 already received"),
  );
});

it("keeps an interrupted scan open and explains safe retry", async () => {
  mock.mutate.mockRejectedValue(new Error("Interrupted"));
  const onClose = vi.fn();
  render(<SeedScheduleDialog schedule={schedule} onClose={onClose} />);
  await userEvent.click(
    screen.getByRole("button", { name: "Grant missing credits" }),
  );
  await waitFor(() =>
    expect(mock.error).toHaveBeenCalledWith(
      expect.stringContaining("Retry safely"),
    ),
  );
  expect(onClose).not.toHaveBeenCalled();
});

it("uses the frozen period amount when only its legacy micros are available", () => {
  render(
    <SeedScheduleDialog
      schedule={{
        ...schedule,
        current_period: {
          ...schedule.current_period!,
          amount: undefined,
          amount_micros: 25_000_000,
        },
      }}
      onClose={vi.fn()}
    />,
  );
  expect(screen.getByText(/Give 25 credits/)).toBeInTheDocument();
});
