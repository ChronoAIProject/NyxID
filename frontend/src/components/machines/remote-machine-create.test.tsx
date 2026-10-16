import type { ReactNode } from "react";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RemoteMachineCreate } from "./remote-machine-create";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { getRemoteMachineIntent } from "@/lib/remote-machine-intent";
import {
  remoteMachineFixture,
  remoteMachinePlatformFixture,
  remoteMachineOperationFixture,
  remoteMachinePaidQuoteFixture,
} from "@/test/remote-machine-fixture";

const state = vi.hoisted(() => ({
  paid: false,
  eligible: true,
  available: true,
  platformReady: true,
  validUntil: "2099-01-01T00:00:00Z",
  walletOwner: "owner",
  entitlementError: null as Error | null,
  walletError: null as Error | null,
  create: vi.fn(),
  quote: vi.fn(),
  refetch: vi.fn(),
  created: vi.fn(),
  close: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { children: ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({
    data: {
      agents: [
        {
          id: "specialist-1",
          name: "Researcher",
          kind: "specialist",
          status: "active",
          owner_kind: "user",
          owner_id: "owner",
        },
        {
          id: "org-agent",
          name: "Org agent",
          kind: "specialist",
          status: "active",
          owner_kind: "org",
          owner_id: "org",
        },
      ],
    },
  }),
}));
vi.mock("@/hooks/use-machines", () => ({
  useRemoteMachineEntitlement: () => ({
    data: {
      eligible: state.eligible,
      free_slot_available: state.available,
      platform: {
        ...remoteMachinePlatformFixture,
        configuration_ready: state.platformReady,
        blockers: state.platformReady ? [] : ["storage_class_missing"],
      },
    },
    error: state.entitlementError,
    refetch: state.refetch,
  }),
  useRemoteMachineQuote: () => ({
    mutate: state.quote,
    data: state.paid
      ? { ...remoteMachinePaidQuoteFixture, valid_until: state.validUntil }
      : {
          valid_until: state.validUntil,
          platform: {
            ...remoteMachinePlatformFixture,
            configuration_ready: state.platformReady,
            blockers: state.platformReady ? [] : ["storage_class_missing"],
          },
          profile: remoteMachineFixture.profile,
          policy: {
            policy_version: "remote-machine-mvp",
            data_policy_version: "remote-machine-data",
          },
        },
  }),
  useCreateRemoteMachine: () => ({
    mutateAsync: state.create,
    isPending: false,
  }),
}));
vi.mock("@/hooks/use-billing", () => ({
  useBillingWallet: () => ({
    data: { owner_id: state.walletOwner, available_credits: 12 },
    error: state.walletError,
  }),
}));
const auth = useAuthStore.getState();
beforeEach(() => {
  useAuthStore.setState({
    user: {
      ...auth.user!,
      id: "owner",
      capabilities: { enabled_features: ["experimental:remote-machines"] },
    },
    isAuthenticated: true,
    isLoading: false,
  });
  sessionStorage.clear();
  Object.assign(state, {
    platformReady: true,
    paid: false,
    eligible: true,
    available: true,
    validUntil: "2099-01-01T00:00:00Z",
    walletOwner: "owner",
    entitlementError: null,
    walletError: null,
  });
  state.create.mockResolvedValue({
    machine: remoteMachineFixture,
    operation: remoteMachineOperationFixture,
    replayed: false,
  });
});
afterEach(() => {
  cleanup();
  useAuthStore.setState(auth);
  sessionStorage.clear();
  vi.resetAllMocks();
});
const show = () => render(<RemoteMachineCreate onCreated={state.created} />);
const accept = () =>
  userEvent.click(
    screen.getByRole("checkbox", { name: /I accept the shared environment/ }),
  );

it("requires policy acceptance and sends only explicit free-slot creation with durable intent", async () => {
  show();
  expect(state.quote).toHaveBeenCalledWith({
    free_slot: true,
    profile_version: "remote-machine",
  });
  expect(screen.getByText(/Profile: remote-machine/)).toBeInTheDocument();
  expect(
    screen.getByText(
      /Temporary storage is managed by the deployment environment/,
    ),
  ).toBeInTheDocument();
  expect(screen.queryByText(/2 GiB temporary storage/)).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Create free remote machine" }),
  ).toBeDisabled();
  expect(
    screen.getByText(/Account wallet available: 12 Credits/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/Paid creation requires a live synchronized quote/),
  ).toBeInTheDocument();
  await accept();
  await userEvent.click(
    screen.getByRole("button", { name: "Create free remote machine" }),
  );
  await waitFor(() => expect(state.create).toHaveBeenCalledTimes(1));
  expect(state.create.mock.calls[0]![0]).toEqual({
    body: {
      name: "my-remote-machine",
      capabilities: ["shell", "files"],
      specialist_id: null,
      free_slot: true,
      profile_version: "remote-machine",
      policy_version: "remote-machine-mvp",
      data_policy_version: "remote-machine-data",
      acknowledge_shared_environment: true,
    },
    idempotencyKey: expect.any(String),
  });
  expect(state.created).toHaveBeenCalledWith("machine-1", "operation-1");
  expect(sessionStorage.length).toBe(0);
});

it("retries the same key and body after an unknown network result", async () => {
  state.create.mockRejectedValueOnce(new Error("Response lost"));
  show();
  await accept();
  await userEvent.click(
    screen.getByRole("button", { name: "Create free remote machine" }),
  );
  await screen.findByText("Response lost");
  expect(sessionStorage.length).toBe(1);
  await userEvent.click(
    screen.getByRole("button", { name: "Retry saved create request" }),
  );
  await waitFor(() => expect(state.create).toHaveBeenCalledTimes(2));
  expect(state.create.mock.calls[1]![0]).toEqual(
    state.create.mock.calls[0]![0],
  );
});

it("can recover an existing intent after reload even if a slot is now occupied and fresh preflight expired", async () => {
  const saved = getRemoteMachineIntent("owner", "create", {
    free_slot: true,
    policy_version: "remote-machine-mvp",
    data_policy_version: "remote-machine-data",
    acknowledge_shared_environment: true,
  });
  state.available = false;
  state.validUntil = "2020-01-01T00:00:00Z";
  show();
  await accept();
  await userEvent.click(
    screen.getByRole("button", { name: "Retry saved create request" }),
  );
  await waitFor(() =>
    expect(state.create).toHaveBeenCalledWith({
      body: {
        ...saved.body,
        profile_version: "remote-machine",
        name: "my-remote-machine",
        capabilities: ["shell", "files"],
        specialist_id: null,
      },
      idempotencyKey: saved.idempotencyKey,
    }),
  );
});

it("blocks a fresh request on expiry and never silently switches an occupied slot to paid", async () => {
  state.available = false;
  state.validUntil = "2020-01-01T00:00:00Z";
  show();
  await accept();
  expect(
    screen.getByRole("button", { name: "Create free remote machine" }),
  ).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Refresh preflight" }),
  ).toBeInTheDocument();
  expect(state.create).not.toHaveBeenCalled();
});

it("does not display a cached wallet belonging to a different owner", () => {
  state.walletOwner = "another-owner";
  show();
  expect(
    screen.getByText(/Account wallet available: Unavailable/),
  ).toBeInTheDocument();
  expect(screen.queryByText(/12 Credits/)).not.toBeInTheDocument();
});

it("does not treat stale eligibility or funds as a successful live preflight", async () => {
  state.entitlementError = new Error("Forbidden");
  state.walletError = new Error("Unavailable");
  show();
  await accept();
  expect(
    screen.getByRole("button", { name: "Create free remote machine" }),
  ).toBeDisabled();
  expect(
    screen.getByText(/Account wallet available: Unavailable/),
  ).toBeInTheDocument();
  expect(state.create).not.toHaveBeenCalled();
});

it("does not navigate or clear the original intent after an account changes during creation", async () => {
  let resolve!: (value: unknown) => void;
  state.create.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  show();
  await accept();
  await userEvent.click(
    screen.getByRole("button", { name: "Create free remote machine" }),
  );
  await waitFor(() => expect(state.create).toHaveBeenCalledTimes(1));
  await act(async () => {
    useAuthStore.setState({ user: { ...auth.user!, id: "different-owner" } });
    resolve({
      machine: remoteMachineFixture,
      operation: remoteMachineOperationFixture,
      replayed: false,
    });
  });
  expect(state.created).not.toHaveBeenCalled();
  expect(state.close).not.toHaveBeenCalled();
  expect(sessionStorage.length).toBe(1);
});

it("blocks new admission and explains platform blockers without claiming physical acceptance", async () => {
  state.platformReady = false;
  show();
  await accept();
  expect(
    screen.getByRole("button", { name: "Create free remote machine" }),
  ).toBeDisabled();
  expect(screen.getAllByText(/storage class missing/i).length).toBeGreaterThan(
    0,
  );
  expect(
    screen.getAllByText(/Kubernetes native lifecycle observations/).length,
  ).toBeGreaterThan(0);
  expect(state.create).not.toHaveBeenCalled();
});

it("collects personal hosted configuration and enforces browser before computer", async () => {
  show();
  expect(screen.getByLabelText("Machine name")).toHaveValue(
    "my-remote-machine",
  );
  expect(screen.getByText(/Personal account/)).toBeInTheDocument();
  expect(screen.getByRole("checkbox", { name: /^Commands/ })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: /^Files/ })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: /^Browser/ })).not.toBeChecked();
  expect(screen.getByRole("checkbox", { name: /^Computer/ })).toBeDisabled();
  await userEvent.click(screen.getByRole("checkbox", { name: /^Browser/ }));
  await userEvent.click(screen.getByRole("checkbox", { name: /^Computer/ }));
  await userEvent.click(screen.getByRole("checkbox", { name: /^Browser/ }));
  expect(screen.getByRole("checkbox", { name: /^Computer/ })).not.toBeChecked();
  expect(screen.getByRole("checkbox", { name: /^Computer/ })).toBeDisabled();
});
it("sends selected name, capabilities and specialist and freezes them for lost-response replay", async () => {
  state.create.mockRejectedValueOnce(new Error("Response lost"));
  show();
  await userEvent.clear(screen.getByLabelText("Machine name"));
  await userEvent.type(screen.getByLabelText("Machine name"), "research");
  await userEvent.click(screen.getByRole("checkbox", { name: /^Browser/ }));
  await userEvent.click(screen.getByRole("checkbox", { name: /^Computer/ }));
  await userEvent.click(
    screen.getByRole("combobox", {
      name: "Select a specialist for this machine",
    }),
  );
  expect(
    screen.queryByRole("option", { name: "Org agent" }),
  ).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("option", { name: "Researcher" }));
  await accept();
  await userEvent.click(
    screen.getByRole("button", { name: "Create free remote machine" }),
  );
  await screen.findByText("Response lost");
  expect(state.create.mock.calls[0]![0].body).toMatchObject({
    name: "research",
    capabilities: ["shell", "files", "browser", "computer"],
    specialist_id: "specialist-1",
  });
  expect(screen.getByLabelText("Machine name")).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Review paid creation" }),
  ).toBeDisabled();
  await userEvent.click(
    screen.getByRole("button", { name: "Retry saved create request" }),
  );
  await waitFor(() => expect(state.create).toHaveBeenCalledTimes(2));
  expect(state.create.mock.calls[1]![0]).toEqual(
    state.create.mock.calls[0]![0],
  );
});

it("preserves configuration through free and paid quote review and replays a paid request after reload", async () => {
  state.create.mockRejectedValueOnce(new Error("Paid response lost"));
  const view = show();
  await userEvent.clear(screen.getByLabelText("Machine name"));
  await userEvent.type(screen.getByLabelText("Machine name"), "paid-research");
  await userEvent.click(screen.getByRole("checkbox", { name: /^Browser/ }));
  await userEvent.click(
    screen.getByRole("combobox", {
      name: "Select a specialist for this machine",
    }),
  );
  await userEvent.click(screen.getByRole("option", { name: "Researcher" }));
  await accept();
  state.paid = true;
  await userEvent.click(
    screen.getByRole("button", { name: "Review paid creation" }),
  );
  expect(screen.getByLabelText("Machine name")).toHaveValue("paid-research");
  expect(screen.getByRole("checkbox", { name: /^Browser/ })).toBeChecked();
  expect(
    screen.getByRole("checkbox", { name: /I accept this exact rate/ }),
  ).not.toBeChecked();
  expect(
    screen.getByRole("button", { name: "Accept quote and create" }),
  ).toBeDisabled();
  state.paid = false;
  await userEvent.click(
    screen.getByRole("button", { name: "Review free slot" }),
  );
  expect(screen.getByLabelText("Machine name")).toHaveValue("paid-research");
  state.paid = true;
  await userEvent.click(
    screen.getByRole("button", { name: "Review paid creation" }),
  );
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I accept this exact rate/ }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Accept quote and create" }),
  );
  await screen.findByText("Paid response lost");
  const submitted = state.create.mock.calls[0]![0];
  expect(submitted.body).toMatchObject({
    name: "paid-research",
    capabilities: ["shell", "files", "browser"],
    specialist_id: "specialist-1",
    free_slot: false,
    quote_id: "quote-one",
    accept_account_balance: true,
  });
  view.unmount();
  state.validUntil = "2020-01-01T00:00:00Z";
  show();
  expect(screen.getByLabelText("Machine name")).toHaveValue("paid-research");
  expect(screen.getByLabelText("Machine name")).toBeDisabled();
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I accept this exact rate/ }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Retry saved create request" }),
  );
  await waitFor(() => expect(state.create).toHaveBeenCalledTimes(2));
  expect(state.create.mock.calls[1]![0]).toEqual(submitted);
});

it("requires a valid name and at least one enabled capability", async () => {
  show();
  await accept();
  await userEvent.click(screen.getByRole("checkbox", { name: /^Commands/ }));
  await userEvent.click(screen.getByRole("checkbox", { name: /^Files/ }));
  expect(
    screen.getByRole("button", { name: "Create free remote machine" }),
  ).toBeDisabled();
  await userEvent.click(screen.getByRole("checkbox", { name: /^Commands/ }));
  await userEvent.clear(screen.getByLabelText("Machine name"));
  await userEvent.type(screen.getByLabelText("Machine name"), "Invalid name");
  await userEvent.click(
    screen.getByRole("button", { name: "Create free remote machine" }),
  );
  expect(
    await screen.findByText("Use 1–64 lowercase letters, numbers or hyphens."),
  ).toBeInTheDocument();
  expect(state.create).not.toHaveBeenCalled();
});
it("keeps selected configuration when a rejected intent is explicitly reset for review", async () => {
  state.create.mockRejectedValueOnce(
    new ApiError(409, {
      error_code: 1,
      error: "conflict",
      message: "Review required",
    }),
  );
  show();
  await userEvent.clear(screen.getByLabelText("Machine name"));
  await userEvent.type(screen.getByLabelText("Machine name"), "preserved-name");
  await userEvent.click(screen.getByRole("checkbox", { name: /^Browser/ }));
  await accept();
  await userEvent.click(
    screen.getByRole("button", { name: "Create free remote machine" }),
  );
  await screen.findByText("Review required");
  expect(screen.getByLabelText("Machine name")).toBeDisabled();
  await userEvent.click(
    screen.getByRole("button", {
      name: "Discard saved request and review again",
    }),
  );
  expect(screen.getByLabelText("Machine name")).toBeEnabled();
  expect(screen.getByLabelText("Machine name")).toHaveValue("preserved-name");
  expect(screen.getByRole("checkbox", { name: /^Browser/ })).toBeChecked();
  expect(
    screen.getByRole("checkbox", { name: /I accept the shared environment/ }),
  ).not.toBeChecked();
  await accept();
  await userEvent.click(
    screen.getByRole("button", { name: "Create free remote machine" }),
  );
  await waitFor(() => expect(state.create).toHaveBeenCalledTimes(2));
  expect(state.create.mock.calls[1]![0].body).toEqual(
    state.create.mock.calls[0]![0].body,
  );
  expect(state.create.mock.calls[1]![0].idempotencyKey).not.toBe(
    state.create.mock.calls[0]![0].idempotencyKey,
  );
});

it("does not mount the hosted form or request preflight while the personal flag is absent", () => {
  useAuthStore.setState({
    user: { id: "owner" } as never,
    isAuthenticated: true,
    isLoading: false,
  });
  show();
  expect(
    screen.queryByRole("form", { name: "NyxID hosted machine configuration" }),
  ).not.toBeInTheDocument();
  expect(state.quote).not.toHaveBeenCalled();
  expect(state.create).not.toHaveBeenCalled();
});

// useFeature still reads the actual auth store; its inactive org query is a double.
vi.mock("@/hooks/use-orgs", () => ({ useOrg: () => ({ data: undefined }) }));

it.each(["off", "loading", "account"])(
  "unmounts an open create form on %s without a new quote or create",
  async (change) => {
    show();
    await accept();
    expect(state.quote).toHaveBeenCalledTimes(1);
    act(() =>
      useAuthStore.setState(
        change === "loading"
          ? { isLoading: true }
          : {
              user: {
                id: change === "account" ? "another-owner" : "owner",
                capabilities: { enabled_features: [] },
              } as never,
            },
      ),
    );
    expect(
      screen.queryByRole("form", {
        name: "NyxID hosted machine configuration",
      }),
    ).not.toBeInTheDocument();
    expect(state.quote).toHaveBeenCalledTimes(1);
    expect(state.create).not.toHaveBeenCalled();
  },
);
