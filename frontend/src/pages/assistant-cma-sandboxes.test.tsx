import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { CmaSandbox } from "@/schemas/nyxbot-cma-sandbox";
import { AssistantCmaSandboxesPage } from "./assistant-cma-sandboxes";

const mocks = vi.hoisted(() => ({
  enabled: true,
  list: vi.fn(),
  rename: vi.fn(),
  command: vi.fn(),
  retire: vi.fn(),
  claim: vi.fn(),
  unregistered: vi.fn(),
}));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => mocks.enabled,
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    params,
    children,
    ...props
  }: {
    to: string;
    params?: Record<string, string>;
    children: React.ReactNode;
  }) => (
    <a
      href={params ? to.replace("$sandboxId", params.sandboxId ?? "") : to}
      {...props}
    >
      {children}
    </a>
  ),
}));
vi.mock("@/hooks/use-nyxbot-cma-sandboxes", () => ({
  useCmaSandboxes: mocks.list,
  useRenameCmaSandbox: () => ({ mutateAsync: mocks.rename }),
  useCmaSandboxCommand: () => ({ mutateAsync: mocks.command }),
  useRetireCmaSandbox: () => ({ mutateAsync: mocks.retire }),
  useClaimCmaSandbox: () => ({ mutateAsync: mocks.claim }),
  useUnregisteredCmaSandboxes: mocks.unregistered,
}));
const row: CmaSandbox = {
  id: "local",
  label: "Personal workspace",
  cma_sandbox_id: "sbx-one",
  generation: 2,
  occupies_account_slot: true,
  admission_state: "associated",
  observation_state: "ready",
  resource_state: "ready",
  machine_included: true,
  cpu_millis: 4000,
  memory_mib: 8192,
  last_activity_at: null,
  last_observed: "2026-11-04T00:00:00Z",
  retire_requested_at: null,
  retired_at: null,
  billing: { provider: "cma", cost_status: "unknown" },
};
beforeEach(() => {
  vi.clearAllMocks();
  mocks.enabled = true;
  mocks.list.mockReturnValue({
    data: { sandboxes: [row], conflict: false, inventory_complete: true },
    refetch: vi.fn(),
  });
  mocks.unregistered.mockReturnValue({ data: { sandboxes: [] } });
  mocks.retire.mockResolvedValue({
    sandbox: { ...row, admission_state: "retiring" },
    pending: true,
  });
  mocks.command.mockResolvedValue({ sandbox: row, pending: true });
});

it("renders the environment with real capacity and no Agent or Desktop management", () => {
  render(<AssistantCmaSandboxesPage />);
  expect(screen.getByText("4 vCPU")).toBeInTheDocument();
  expect(screen.getByText("8 GiB")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /view details/i })).toHaveAttribute(
    "href",
    "/assistant/sandboxes/local",
  );
  expect(
    screen.queryByRole("button", { name: /desktop|create agent/i }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByText(/price and accrued charges are unknown/i),
  ).toBeInTheDocument();
});

it("does not create anything in the empty state", () => {
  mocks.list.mockReturnValue({
    data: { sandboxes: [], conflict: false, inventory_complete: true },
    refetch: vi.fn(),
  });
  render(<AssistantCmaSandboxesPage />);
  expect(screen.getByText("Room for your next task")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /ask nyxbot/i })).toBeInTheDocument();
  expect(mocks.command).not.toHaveBeenCalled();
});

it("keeps an unknown creation occupied and does not offer retirement of an unverified target", () => {
  mocks.list.mockReturnValue({
    data: {
      sandboxes: [
        {
          ...row,
          cma_sandbox_id: null,
          admission_state: "unknown",
          observation_state: "unknown",
          cpu_millis: null,
          memory_mib: null,
        },
      ],
      conflict: false,
      inventory_complete: false,
    },
    refetch: vi.fn(),
  });
  render(<AssistantCmaSandboxesPage />);
  expect(screen.getByText(/allocation remains reserved/i)).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Retire environment" }),
  ).toBeDisabled();
  expect(screen.queryByText("4 vCPU")).not.toBeInTheDocument();
});

it("shows every conflicting environment rather than selecting one", () => {
  mocks.list.mockReturnValue({
    data: {
      sandboxes: [
        row,
        {
          ...row,
          id: "second",
          label: "Imported workspace",
          cma_sandbox_id: "sbx-two",
          occupies_account_slot: false,
        },
      ],
      conflict: true,
      inventory_complete: true,
    },
    refetch: vi.fn(),
  });
  render(<AssistantCmaSandboxesPage />);
  expect(screen.getByText("Imported workspace")).toBeInTheDocument();
  expect(screen.getByText(/new environments are blocked/i)).toBeInTheDocument();
  expect(
    screen.getAllByRole("button", { name: "Retire environment" }),
  ).toHaveLength(2);
});

it("requires the exact name for retirement and treats 202 as pending", async () => {
  render(<AssistantCmaSandboxesPage />);
  fireEvent.click(screen.getByRole("button", { name: "Retire environment" }));
  const dialog = screen.getByRole("dialog");
  const confirm = within(dialog).getByRole("button", {
    name: "Retire environment",
  });
  expect(confirm).toBeDisabled();
  fireEvent.change(within(dialog).getByLabelText(/type.*confirm/i), {
    target: { value: "Personal workspace" },
  });
  fireEvent.click(confirm);
  await waitFor(() =>
    expect(mocks.retire).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "local",
        generation: 2,
        idempotencyKey: expect.any(String),
      }),
    ),
  );
  expect(screen.queryByText("Environment retired")).not.toBeInTheDocument();
});

it("gates direct page access when the feature is off", () => {
  mocks.enabled = false;
  render(<AssistantCmaSandboxesPage />);
  expect(screen.getByText(/not enabled for this account/i)).toBeInTheDocument();
  expect(screen.queryByText("Personal workspace")).not.toBeInTheDocument();
});

it("releases a confirmed no-resource creation without claiming a remote retirement", () => {
  mocks.list.mockReturnValue({
    data: {
      sandboxes: [
        {
          ...row,
          cma_sandbox_id: null,
          admission_state: "retired",
          observation_state: "unknown",
          occupies_account_slot: false,
        },
      ],
      conflict: false,
      inventory_complete: true,
    },
    refetch: vi.fn(),
  });
  render(<AssistantCmaSandboxesPage />);
  expect(screen.getByText("Room for your next task")).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: /link existing/i }),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/allocation remains reserved/i),
  ).not.toBeInTheDocument();
});
