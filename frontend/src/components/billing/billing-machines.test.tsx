import type { ReactNode } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BillingMachines } from "./billing-machines";
import { remoteMachineFixture } from "@/test/remote-machine-fixture";
import { ApiError } from "@/lib/api-client";

const state = vi.hoisted(() => ({
  query: vi.fn(),
  lookup: vi.fn(),
  error: null as Error | null,
  lookupError: null as Error | null,
  lookupMachineId: "00000000-0000-4000-8000-000000000001",
}));
vi.mock("@/hooks/use-machines", () => ({
  useRemoteMachines: () => {
    state.query();
    return {
      data: state.error ? undefined : [remoteMachineFixture],
      error: state.error,
    };
  },
  useRemoteMachine: (id: string | undefined) => {
    state.lookup(id);
    return {
      data: id
        ? {
            ...remoteMachineFixture,
            node_id: state.lookupMachineId,
            resource_state: "deleted",
            billing_state: "closed",
          }
        : undefined,
      error: state.lookupError,
      refetch: vi.fn(),
    };
  },
}));
vi.mock("@/components/machines/remote-machine-billing", () => ({
  RemoteMachineBilling: ({ machineId }: { machineId: string }) => (
    <p>Runtime projection for {machineId}</p>
  ),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { children: ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));
beforeEach(() => {
  state.error = null;
  state.lookupError = null;
  state.lookupMachineId = "00000000-0000-4000-8000-000000000001";
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

it("loads machine accounting only when the existing Billing panel is explicitly opened", async () => {
  render(<BillingMachines />);
  expect(screen.getByText(/Temporary storage is managed by the deployment environment/)).toBeInTheDocument();
  expect(screen.queryByText(/2 GiB temporary storage/)).not.toBeInTheDocument();
  expect(state.query).not.toHaveBeenCalled();
  expect(
    screen.getByText(
      /Adding account funds never resumes a machine automatically/,
    ),
  ).toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("button", { name: "Review machine runtime" }),
  );
  await userEvent.click(
    screen.getByRole("combobox", { name: "Machine runtime billing selection" }),
  );
  await userEvent.click(
    screen.getByRole("option", {
      name: "machine-1 · provisioning · machine-1",
    }),
  );
  expect(
    screen.getByText("Runtime projection for machine-1"),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("link", { name: "Manage this machine" }),
  ).toHaveAttribute("href", "/assistant/machines");
});

it("does not turn missing machine management into a zero balance or empty billing history", async () => {
  state.error = new ApiError(404, {
    error: "not_found",
    message: "Not found",
    error_code: 1003,
  });
  render(<BillingMachines />);
  await userEvent.click(
    screen.getByRole("button", { name: "Review machine runtime" }),
  );
  expect(
    screen.getByText(/No runtime balance or accepted rate can be inferred/),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/No current remote machines/),
  ).not.toBeInTheDocument();
});

it("looks up owner-readable deleted Machine billing by its real retained ID", async () => {
  render(<BillingMachines />);
  await userEvent.click(
    screen.getByRole("button", { name: "Review machine runtime" }),
  );
  await userEvent.type(
    screen.getByLabelText("Known Machine ID, including a deleted Machine"),
    state.lookupMachineId,
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Look up retained Machine billing" }),
  );
  expect(state.lookup).toHaveBeenLastCalledWith(state.lookupMachineId);
  expect(
    await screen.findByText(`Runtime projection for ${state.lookupMachineId}`),
  ).toBeInTheDocument();
  expect(screen.getByText(/This Machine is deleted/)).toBeInTheDocument();
  expect(
    screen.getByRole("link", { name: "View retained Machine details" }),
  ).toHaveAttribute("href", "/assistant/machines");
});

it("retains the known-ID lookup when the active list fails without inferring a billing result", async () => {
  state.error = new Error("Active list unavailable");
  state.lookupError = new Error("Not owned");
  render(<BillingMachines />);
  await userEvent.click(
    screen.getByRole("button", { name: "Review machine runtime" }),
  );
  await userEvent.type(
    screen.getByLabelText("Known Machine ID, including a deleted Machine"),
    state.lookupMachineId,
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Look up retained Machine billing" }),
  );
  expect(
    await screen.findByText(/retained Machine record could not be verified/),
  ).toBeInTheDocument();
  expect(screen.queryByText(/Runtime projection for/)).not.toBeInTheDocument();
});

it("does not show billing for a mismatched retained lookup response", async () => {
  render(<BillingMachines />);
  await userEvent.click(
    screen.getByRole("button", { name: "Review machine runtime" }),
  );
  await userEvent.type(
    screen.getByLabelText("Known Machine ID, including a deleted Machine"),
    "00000000-0000-4000-8000-000000000002",
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Look up retained Machine billing" }),
  );
  expect(
    await screen.findByText(/retained Machine record could not be verified/),
  ).toBeInTheDocument();
  expect(screen.queryByText(/Runtime projection for/)).not.toBeInTheDocument();
});

it("validates the retained ID and discloses the agreed fixed and retention policies", async () => {
  render(<BillingMachines />);
  expect(screen.getByText(/New Machine fixed profile: 2 vCPU/)).toHaveTextContent(
    /one free slot with no expiry or monthly runtime limit/,
  );
  expect(
    screen.getByText(/Paid runtime has no per-instance spending cap/),
  ).toHaveTextContent(/Resume is manual/);
  expect(
    screen.getByText(/Paused workspaces are retained indefinitely/),
  ).toHaveTextContent(/Job metadata lasts 30 days/);
  await userEvent.click(
    screen.getByRole("button", { name: "Review machine runtime" }),
  );
  await userEvent.type(
    screen.getByLabelText("Known Machine ID, including a deleted Machine"),
    "not-an-id",
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Look up retained Machine billing" }),
  );
  expect(
    await screen.findByText("Enter the retained Machine ID as a UUID."),
  ).toBeInTheDocument();
  expect(state.lookup).not.toHaveBeenCalledWith("not-an-id");
});

vi.mock("@/components/machines/remote-machine-service-usage", () => ({
  RemoteMachineServiceUsage: ({ machineId }: { machineId: string }) => (
    <div data-testid="machine-service-usage">{machineId}</div>
  ),
}));
