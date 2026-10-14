import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { ProvisioningControls, SetupStatusBadge } from "./platform-setup-detail";
import {
  blankFacts,
  type PlatformSetup,
  type SetupApproval,
} from "@/schemas/platform-onboarding";
const state = vi.hoisted(() => ({
  review: null as unknown,
  actor: "client" as string | undefined,
  mutate: vi.fn(),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (value: { user: { id: string } | null }) => unknown) =>
    select({ user: state.actor ? { id: state.actor } : null }),
}));
vi.mock("@/hooks/use-platform-onboarding", () => ({
  useSetupApproval: () => ({ data: state.review }),
  useSetupProvisioning: () => ({ mutate: state.mutate, isPending: false }),
}));
const setup: PlatformSetup = {
  id: "setup",
  template: "restaurant-starter",
  template_version: 1,
  agent_id: null,
  channel_bot_id: null,
  creation_request_id: null,
  observed_telegram_bot_id: null,
  publication_revision: 2,
  approved: false,
  expires_at: null,
  updated_at: "2026-10-14T00:00:00Z",
  revision: 2,
  state: "awaiting_child_review",
  delivery_tested: false,
  facts: { ...blankFacts(), name: "Reviewed cafe", timezone: "UTC", faq: [] },
  limits: {
    daily_credits: "1",
    turns_per_hour: 10,
    concurrent_turns: 1,
    max_input_chars: 2048,
    max_output_tokens: 512,
  },
};
const review: SetupApproval = {
  setup,
  digest: "reviewed-digest",
  reviewed_revision: 2,
  request_revision: 3,
  child_bot_id: 900,
  child_username: "ClientBot",
  manager_revision: 1,
  model_id: "exact-model",
  connector_grants: [],
  agent_id: "agent",
  creation_request_id: "request",
  model_service_id: "service",
  funding_owner_user_id: "client",
};
beforeEach(() => {
  state.actor = "client";
  state.review = { ...review };
  state.mutate.mockClear();
});
it("refuses approval when the fetched request changes after review", () => {
  const page = render(<ProvisioningControls setup={setup} />);
  fireEvent.click(screen.getByRole("checkbox"));
  state.review = { ...review, request_revision: 4 };
  page.rerender(<ProvisioningControls setup={setup} />);
  expect(
    screen.getByRole("button", { name: "Approve reviewed setup" }),
  ).toBeDisabled();
  expect(state.mutate).not.toHaveBeenCalled();
});
it("submits the captured exact child review", () => {
  render(<ProvisioningControls setup={setup} />);
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(
    screen.getByRole("button", { name: "Approve reviewed setup" }),
  );
  expect(state.mutate).toHaveBeenCalledWith({
    action: "child-approve",
    review,
  });
});
it("does not activate before independent customer receipt", () => {
  render(
    <ProvisioningControls setup={{ ...setup, state: "ready_for_test" }} />,
  );
  expect(
    screen.getByRole("button", { name: "Activate customer replies" }),
  ).toBeDisabled();
});

it("renders safely and refuses approval without an authenticated actor", () => {
  state.actor = undefined;
  render(<ProvisioningControls setup={setup} />);
  fireEvent.click(screen.getByRole("checkbox"));
  expect(
    screen.getByRole("button", { name: "Approve reviewed setup" }),
  ).toBeDisabled();
  expect(state.mutate).not.toHaveBeenCalled();
});

it("hides creation links after a revision or account change", () => {
  const current = { ...setup, state: "provisioning" as const };
  const page = render(<ProvisioningControls setup={current} />);
  fireEvent.click(
    screen.getByRole("button", { name: "Prepare Telegram creation link" }),
  );
  const callbacks = state.mutate.mock.calls[0]![1] as {
    onSuccess: (value: { url: string }) => void;
  };
  act(() =>
    callbacks.onSuccess({ url: "https://t.me/ManagerBot?start=fixture" }),
  );
  expect(
    screen.getByRole("link", { name: "Open Telegram manager" }),
  ).toBeInTheDocument();
  state.actor = "another-client";
  page.rerender(<ProvisioningControls setup={current} />);
  expect(
    screen.queryByRole("link", { name: "Open Telegram manager" }),
  ).not.toBeInTheDocument();
  state.actor = "client";
  page.rerender(<ProvisioningControls setup={{ ...current, revision: 3 }} />);
  expect(
    screen.queryByRole("link", { name: "Open Telegram manager" }),
  ).not.toBeInTheDocument();
});

it("shows the captured saved facts and limits instead of unsaved editor content", () => {
  render(<ProvisioningControls setup={setup} />);
  expect(screen.getByText("Reviewed cafe")).toBeInTheDocument();
  expect(screen.getByText(/Daily cap 1 credit/)).toBeInTheDocument();
  expect(
    screen.getByText(/Creates or resumes only your reviewed specialist/),
  ).toBeInTheDocument();
});

it("shows Paused for unavailable effective runtime despite stored Active", () => {
  const page = render(<SetupStatusBadge setup={{ ...setup, state: "active", effective_state: "paused" }} />);
  expect(screen.getByText("Paused")).toBeInTheDocument();
  expect(screen.queryByText("Active")).not.toBeInTheDocument();
  page.rerender(<SetupStatusBadge setup={{ ...setup, state: "active", effective_state: "active" }} />);
  expect(screen.getByText("Active")).toBeInTheDocument();
});
