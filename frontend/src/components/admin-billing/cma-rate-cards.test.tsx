import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CmaRateCards } from "./cma-rate-cards";
const mock = vi.hoisted(() => ({ publish: vi.fn() }));
vi.mock("@/hooks/use-cma-rate-cards", () => ({
  useCmaRateCards: () => ({
    data: [
      {
        id: "v1",
        version: 1,
        effective_from: "2026-01-01T00:00:00Z",
        sync_status: "synced",
        rates: {
          sandbox_cpu_millis_seconds: "0.00000001",
          sandbox_memory_mib_seconds: "0.000000001",
          sandbox_disk_gib_seconds: "0.00000004",
        },
      },
    ],
    isLoading: false,
    error: null,
  }),
  usePublishCmaRateCard: () => ({
    mutate: mock.publish,
    isPending: false,
    error: null,
  }),
}));
describe("CMA rate configuration", () => {
  beforeEach(() => mock.publish.mockReset());
  it("keeps operator history read-only", () => {
    render(<CmaRateCards canWrite={false} />);
    expect(screen.getByText("0.00000001")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Publish rate version" }),
    ).not.toBeInTheDocument();
  });
  it("publishes exact strings with the current version and prospective timestamp", async () => {
    render(<CmaRateCards canWrite />);
    const user = userEvent.setup();
    const cpu = screen.getByLabelText("CPU (credits per millicore-second)");
    await user.clear(cpu);
    await user.type(cpu, "0.000000020001");
    const future = new Date(Date.now() + 3600_000);
    const local = new Date(
      future.getTime() - future.getTimezoneOffset() * 60_000,
    )
      .toISOString()
      .slice(0, 19);
    fireEvent.change(screen.getByLabelText("Effective from (local time)"), {
      target: { value: local },
    });
    await user.click(
      screen.getByRole("button", { name: "Publish rate version" }),
    );
    expect(mock.publish).toHaveBeenCalledWith(
      expect.objectContaining({
        expected_version: 1,
        effective_from: new Date(local).toISOString(),
        rates: expect.objectContaining({
          sandbox_cpu_millis_seconds: "0.000000020001",
        }),
      }),
    );
  });
  it("rejects past changes before sending a write", async () => {
    render(<CmaRateCards canWrite />);
    fireEvent.change(screen.getByLabelText("Effective from (local time)"), {
      target: { value: "2020-01-01T00:00:00" },
    });
    await userEvent.click(
      screen.getByRole("button", { name: "Publish rate version" }),
    );
    expect(
      await screen.findByText(/at least 301 seconds from now/),
    ).toBeInTheDocument();
    expect(mock.publish).not.toHaveBeenCalled();
  });
});
