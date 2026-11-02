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
  entitlementPending: false,
  reason: null as string | null,
  machineId: null as string | null,
  platformReady: true,
  validUntil: "2099-01-01T00:00:00Z",
  walletOwner: "owner",
  entitlementError: null as Error | null,
  quoteError: null as Error | null,
  walletError: null as Error | null,
  create: vi.fn(),
  quote: vi.fn(),
  refetch: vi.fn(),
  statusRead: vi.fn(),
  created: vi.fn(),
  close: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    search,
  }: {
    children: ReactNode;
    to: string;
    search?: { machine: string };
  }) => (
    <a href={search ? `${to}?machine=${search.machine}` : to}>{children}</a>
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
    data: state.entitlementPending
      ? undefined
      : {
          eligible: state.eligible,
          free_slot_available: state.available,
          reason: state.reason,
          machine_id: state.machineId,
          platform: {
            ...remoteMachinePlatformFixture,
            configuration_ready: state.platformReady,
            blockers: state.platformReady ? [] : ["storage_class_missing"],
          },
        },
    error: state.entitlementError,
    isPending: state.entitlementPending,
    refetch: state.refetch,
  }),
  useRemoteMachines: () => ({ refetch: state.statusRead }),
  useRemoteMachine: () => ({
    data: state.machineId
      ? { name: "existing-workspace", resource_state: "paused" }
      : undefined,
  }),
  useRemoteMachineQuote: () => ({
    mutate: state.quote,
    error: state.quoteError,
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
            ...remoteMachinePaidQuoteFixture.policy,
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
    entitlementPending: false,
    reason: null,
    machineId: null,
    validUntil: "2099-01-01T00:00:00Z",
    walletOwner: "owner",
    entitlementError: null,
    quoteError: null,
    walletError: null,
  });
  state.refetch.mockImplementation(async () => ({
    data: { machine_id: state.machineId },
    error: null,
  }));
  state.statusRead.mockResolvedValue({
    data: { machines: [], complete: true },
    error: null,
  });
  state.create.mockResolvedValue({
    machine: remoteMachineFixture,
    operation: remoteMachineOperationFixture,
    replayed: false,
  });
});

it("reviews three hosted defaults and requires fresh explicit separation consent after a choice change", async () => {
  show();
  const choice = screen.getByRole("combobox", { name: "Agent separation" });
  expect(
    screen.getByText(/Automatic.*waits for confirmed separation support/),
  ).toBeVisible();
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I accept the shared environment/ }),
  );
  expect(
    screen.getByRole("button", { name: "Create free remote machine" }),
  ).toBeDisabled();
  await userEvent.click(
    screen.getByRole("checkbox", {
      name: /I accept the selected Agent separation/,
    }),
  );
  await userEvent.click(choice);
  await userEvent.click(
    screen.getByRole("option", { name: "Explicit shared legacy access" }),
  );
  expect(
    screen.getByRole("checkbox", {
      name: /I accept the selected Agent separation/,
    }),
  ).not.toBeChecked();
  expect(screen.getByText(/shared OS/i)).toBeVisible();
  await userEvent.click(
    screen.getByRole("checkbox", {
      name: /I accept the selected Agent separation/,
    }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Create free remote machine" }),
  );
  await waitFor(() => expect(state.create).toHaveBeenCalledOnce());
  expect(state.create.mock.calls[0]![0].body).toMatchObject({
    separation_choice: "shared_legacy",
    separation_policy_version: "remote-machine-separation-v1",
    acknowledge_separation: true,
    capabilities: ["files", "shell"],
  });
});

it("keeps capability changes free of consent errors and clears explicitly invalidated acknowledgements", async () => {
  show();
  await userEvent.click(screen.getByRole("checkbox", { name: /^Browser/ }));
  expect(
    screen.queryByText(
      "Review and accept the selected Agent separation policy.",
    ),
  ).not.toBeInTheDocument();
  const separation = screen.getByRole("checkbox", {
    name: /I accept the selected Agent separation/,
  });
  await userEvent.click(separation);
  await userEvent.click(separation);
  await screen.findByText(
    "Review and accept the selected Agent separation policy.",
  );
  await userEvent.click(
    screen.getByRole("checkbox", {
      name: /I accept the selected Agent separation/,
    }),
  );
  await waitFor(() =>
    expect(
      screen.queryByText(
        "Review and accept the selected Agent separation policy.",
      ),
    ).not.toBeInTheDocument(),
  );
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I accept the shared environment/ }),
  );
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I accept the shared environment/ }),
  );
  await screen.findByText(
    "Review and accept the displayed profile, billing and data policies.",
  );
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I accept the shared environment/ }),
  );
  await waitFor(() =>
    expect(
      screen.queryByText(
        "Review and accept the displayed profile, billing and data policies.",
      ),
    ).not.toBeInTheDocument(),
  );
  expect(
    screen.getByRole("button", { name: "Create free remote machine" }),
  ).toBeEnabled();
  expect(state.create).not.toHaveBeenCalled();
});

it.each([false, true])(
  "Require separated checks account context permission %s before creating, without granting tools",
  async (contexts) => {
    useAuthStore.setState({
      user: {
        ...useAuthStore.getState().user!,
        capabilities: {
          enabled_features: [
            "experimental:remote-machines",
            ...(contexts ? ["assistant:machine-contexts"] : []),
          ],
        },
      },
    });
    show();
    await userEvent.click(
      screen.getByRole("combobox", { name: "Agent separation" }),
    );
    await userEvent.click(
      screen.getByRole("option", {
        name: "Require separated workspaces and browsers",
      }),
    );
    await accept();
    const submit = screen.getByRole("button", {
      name: "Create free remote machine",
    });
    if (contexts) {
      expect(submit).toBeEnabled();
      await userEvent.click(submit);
      await waitFor(() => expect(state.create).toHaveBeenCalledOnce());
      expect(state.create.mock.calls[0]![0].body).toMatchObject({
        separation_choice: "require_separated",
        acknowledge_separation: true,
      });
    } else {
      expect(submit).toBeDisabled();
      expect(state.create).not.toHaveBeenCalled();
    }
  },
);
afterEach(() => {
  cleanup();
  useAuthStore.setState(auth);
  sessionStorage.clear();
  vi.resetAllMocks();
});
const show = (allowPaidCreation = true) =>
  render(
    <RemoteMachineCreate
      allowPaidCreation={allowPaidCreation}
      onCreated={state.created}
    />,
  );
const accept = async () => {
  const separation = screen.getByRole("checkbox", {
    name: /I accept the selected Agent separation/,
  });
  if (separation.getAttribute("data-state") !== "checked")
    await userEvent.click(separation);
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I accept the shared environment/ }),
  );
};

it("requires policy acceptance and sends only explicit free-slot creation with durable intent", async () => {
  show(false);
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
    screen.queryByText(/Account wallet available/),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText(/Paid creation requires a live synchronized quote/),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Review paid creation" }),
  ).not.toBeInTheDocument();
  await accept();
  await userEvent.click(
    screen.getByRole("button", { name: "Create free remote machine" }),
  );
  await waitFor(() => expect(state.create).toHaveBeenCalledTimes(1));
  expect(state.create.mock.calls[0]![0]).toEqual({
    body: {
      name: "my-remote-machine",
      capabilities: ["files", "shell"],
      specialist_id: null,
      free_slot: true,
      profile_version: "remote-machine",
      policy_version: "remote-machine-mvp",
      data_policy_version: "remote-machine-data",
      acknowledge_shared_environment: true,
      separation_choice: "automatic",
      separation_policy_version: "remote-machine-separation-v1",
      acknowledge_separation: true,
    },
    idempotencyKey: expect.any(String),
  });
  expect(state.created).toHaveBeenCalledWith("machine-1", "operation-1", null);
  expect(sessionStorage.length).toBe(0);
});

it("shows a free summary and keeps full policies and technical observations in closed disclosures", async () => {
  show(false);
  await act(async () => {});
  expect(
    screen.queryByText(
      "Review and accept the selected Agent separation policy.",
    ),
  ).not.toBeInTheDocument();
  expect(screen.getByText("Available", { exact: true })).toBeVisible();
  expect(
    screen.queryByText("Eligible", { exact: true }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText("Your personal free slot is available."),
  ).not.toBeInTheDocument();
  expect(screen.getByLabelText("Machine specifications")).toHaveTextContent(
    "2 vCPUCompute2 GiBMemory20 GiBWorkspaceLinuxSystem",
  );
  expect(
    screen.getByText(
      /Machine runtime is free. Model and external API usage may be billed separately/,
    ),
  ).toBeVisible();
  const temporaryStorage = screen.getByText(
    /Temporary storage is managed by the deployment environment/,
  );
  const platform = screen.getByText(/Kubernetes native lifecycle observations/);
  expect(temporaryStorage).not.toBeVisible();
  expect(platform).not.toBeVisible();
  expect(
    screen.getByText("Capacity will be checked during creation."),
  ).toBeVisible();
  await userEvent.click(
    screen.getByText("Limits and data policies", { exact: true }),
  );
  expect(temporaryStorage).toBeVisible();
  expect(screen.getByText(/separated workspaces are retained/)).toBeVisible();
  expect(
    screen.getByText(/terminal operation history for 90 days/),
  ).toBeVisible();
  await userEvent.click(screen.getByText("Technical details", { exact: true }));
  expect(platform).toBeVisible();
  expect(screen.getByText(/Namespace quota:/)).toBeVisible();
  await accept();
  expect(
    screen.getByRole("button", { name: "Create free remote machine" }),
  ).toBeEnabled();
  await userEvent.click(
    screen.getByRole("button", { name: "Refresh preflight" }),
  );
  expect(
    screen.getByRole("checkbox", { name: /I accept the shared environment/ }),
  ).not.toBeChecked();
  expect(
    screen.getByRole("checkbox", {
      name: /I accept the selected Agent separation/,
    }),
  ).not.toBeChecked();
});

it("shows occupied status and links to its machine without offering paid creation", async () => {
  state.available = false;
  state.machineId = "occupied-machine";
  show(false);
  expect(screen.getByText("Occupied", { exact: true })).toBeVisible();
  expect(screen.getByText(/existing-workspace.*paused/)).toBeVisible();
  expect(screen.getByRole("link", { name: "View machine" })).toHaveAttribute(
    "href",
    "/assistant/machines?machine=occupied-machine",
  );
  expect(screen.queryByRole("form")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Review paid creation" }),
  ).not.toBeInTheDocument();
  expect(state.create).not.toHaveBeenCalled();
});

it("shows an ineligible reason and disables free creation", async () => {
  state.eligible = false;
  state.reason = "account_suspended";
  show(false);
  expect(screen.getByText("Ineligible", { exact: true })).toBeVisible();
  expect(screen.getByText("account suspended")).toBeVisible();
  await accept();
  expect(
    screen.getByRole("button", { name: "Create free remote machine" }),
  ).toBeDisabled();
});

it("does not advertise an available slot while eligibility is loading or has failed", () => {
  state.entitlementPending = true;
  const view = show(false);
  expect(screen.getByText("Checking…", { exact: true })).toBeVisible();
  expect(
    screen.queryByText("Available", { exact: true }),
  ).not.toBeInTheDocument();
  state.entitlementPending = false;
  state.entitlementError = new Error("Eligibility service unavailable");
  view.rerender(
    <RemoteMachineCreate allowPaidCreation={false} onCreated={state.created} />,
  );
  expect(screen.getByText(/Eligibility is unavailable/)).toBeVisible();
  expect(
    screen.queryByText("Available", { exact: true }),
  ).not.toBeInTheDocument();
});

it("keeps platform blockers and failed preflight visible outside technical details", () => {
  state.platformReady = false;
  state.quoteError = new Error("Quote service unavailable");
  show(false);
  expect(
    screen.getByText(/Preflight is unavailable.*Quote service unavailable/),
  ).toBeVisible();
  expect(
    screen.getByText(/Machine setup is unavailable.*storage class missing/),
  ).toBeVisible();
  expect(
    screen.getByRole("button", { name: "Create free remote machine" }),
  ).toBeDisabled();
});

it.each(["create"])(
  "preserves a saved paid %s request without quoting or replaying it when paid creation is hidden",
  (scope) => {
    getRemoteMachineIntent("owner", scope, {
      free_slot: false,
      quote_id: "saved-quote",
      accept_account_balance: true,
      policy_version: "remote-machine-mvp",
      data_policy_version: "remote-machine-data",
      acknowledge_shared_environment: true,
      separation_choice: "automatic",
      separation_policy_version: "remote-machine-separation-v1",
      acknowledge_separation: true,
    });
    const storageKey = sessionStorage.key(0)!;
    const original = sessionStorage.getItem(storageKey);
    show(false);
    expect(
      screen.getByText(/Paid creation is temporarily unavailable/),
    ).toBeVisible();
    expect(screen.queryByRole("form")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Paid account-balance runtime" }),
    ).not.toBeInTheDocument();
    expect(state.quote).not.toHaveBeenCalled();
    expect(state.create).not.toHaveBeenCalled();
    expect(sessionStorage.getItem(storageKey)).toBe(original);
    expect(sessionStorage.length).toBe(1);
  },
);

it("requires a fresh status read before explicitly retrying the same key and body after an unknown result", async () => {
  state.create.mockRejectedValueOnce(new Error("Response lost"));
  show();
  await accept();
  await userEvent.click(
    screen.getByRole("button", { name: "Create free remote machine" }),
  );
  await screen.findByText("Response lost");
  expect(sessionStorage.length).toBe(1);
  expect(
    screen.getByRole("button", { name: "Retry saved create request" }),
  ).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Check status" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Retry saved create request" }),
    ).toBeEnabled(),
  );
  expect(state.create).toHaveBeenCalledOnce();
  await userEvent.click(
    screen.getByRole("button", { name: "Retry saved create request" }),
  );
  await waitFor(() => expect(state.create).toHaveBeenCalledTimes(2));
  expect(state.create.mock.calls[1]![0]).toEqual(
    state.create.mock.calls[0]![0],
  );
});

const saveUnknownCreate = (owner = "owner") =>
  getRemoteMachineIntent(owner, "create", {
    name: "my-remote-machine",
    capabilities: ["files", "shell"],
    specialist_id: null,
    free_slot: true,
    profile_version: "remote-machine",
    policy_version: "remote-machine-mvp",
    data_policy_version: "remote-machine-data",
    acknowledge_shared_environment: true,
    separation_choice: "automatic",
    separation_policy_version: "remote-machine-separation-v1",
    acknowledge_separation: true,
  });

it("keeps retry blocked while status is pending and consumes each successful observation on dispatch", async () => {
  const saved = saveUnknownCreate();
  let resolve!: (value: unknown) => void;
  state.statusRead.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  state.create.mockRejectedValueOnce(new Error("Response lost again"));
  show();
  await accept();
  await userEvent.click(screen.getByRole("button", { name: "Check status" }));
  const retry = screen.getByRole("button", {
    name: "Retry saved create request",
  });
  expect(retry).toBeDisabled();
  await userEvent.click(retry);
  expect(state.create).not.toHaveBeenCalled();
  await act(async () => {
    resolve({ data: { machines: [], complete: true }, error: null });
  });
  await waitFor(() => expect(retry).toBeEnabled());
  expect(state.create).not.toHaveBeenCalled();
  await userEvent.click(retry);
  await screen.findByText("Response lost again");
  expect(state.create).toHaveBeenCalledWith({
    body: saved.body,
    idempotencyKey: saved.idempotencyKey,
  });
  expect(retry).toBeDisabled();
});

it.each(["entitlement", "machines", "incomplete"])(
  "blocks retry after an unsuccessful %s status observation",
  async (failure) => {
    saveUnknownCreate();
    if (failure === "entitlement")
      state.refetch.mockResolvedValue({
        data: { machine_id: null },
        error: new Error("Status unavailable"),
      });
    else
      state.statusRead.mockResolvedValue({
        data: { machines: [], complete: failure !== "incomplete" },
        error: failure === "machines" ? new Error("Status unavailable") : null,
      });
    show();
    await accept();
    await userEvent.click(screen.getByRole("button", { name: "Check status" }));
    await screen.findByText(/Status could not be confirmed/);
    expect(
      screen.getByRole("button", { name: "Retry saved create request" }),
    ).toBeDisabled();
    expect(state.create).not.toHaveBeenCalled();
  },
);

it("does not carry a successful observation across close and reopen", async () => {
  saveUnknownCreate();
  const view = show();
  await accept();
  await userEvent.click(screen.getByRole("button", { name: "Check status" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Retry saved create request" }),
    ).toBeEnabled(),
  );
  view.unmount();
  show();
  await accept();
  expect(
    screen.getByRole("button", { name: "Retry saved create request" }),
  ).toBeDisabled();
  expect(state.create).not.toHaveBeenCalled();
});

it("does not let an old owner's pending read unlock another owner's saved intent", async () => {
  saveUnknownCreate();
  saveUnknownCreate("different-owner");
  let resolve!: (value: unknown) => void;
  state.statusRead.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  show();
  await accept();
  await userEvent.click(screen.getByRole("button", { name: "Check status" }));
  await act(async () => {
    useAuthStore.setState({
      user: { ...useAuthStore.getState().user!, id: "different-owner" },
    });
  });
  await accept();
  await act(async () => {
    resolve({ data: { machines: [], complete: true }, error: null });
  });
  expect(
    screen.getByRole("button", { name: "Retry saved create request" }),
  ).toBeDisabled();
  expect(state.create).not.toHaveBeenCalled();
});

it("does not let a pending read unlock a replaced intent for the same owner", async () => {
  saveUnknownCreate();
  let resolve!: (value: unknown) => void;
  state.statusRead.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  show();
  await accept();
  await userEvent.click(screen.getByRole("button", { name: "Check status" }));
  sessionStorage.clear();
  saveUnknownCreate();
  await act(async () => {
    resolve({ data: { machines: [], complete: true }, error: null });
  });
  expect(
    screen.getByRole("button", { name: "Retry saved create request" }),
  ).toBeDisabled();
  expect(state.create).not.toHaveBeenCalled();
});

it("routes a freshly observed occupied free slot to its existing machine without replaying create", async () => {
  saveUnknownCreate();
  state.machineId = remoteMachineFixture.node_id;
  state.available = false;
  state.statusRead.mockResolvedValue({
    data: { machines: [remoteMachineFixture], complete: true },
    error: null,
  });
  show();
  await accept();
  await userEvent.click(screen.getByRole("button", { name: "Check status" }));
  await screen.findByText(/Continue with the existing machine/);
  expect(screen.getByRole("link", { name: "View machine" })).toHaveAttribute(
    "href",
    `/assistant/machines?machine=${remoteMachineFixture.node_id}`,
  );
  expect(
    screen.getByRole("button", { name: "Retry saved create request" }),
  ).toBeDisabled();
  expect(state.create).not.toHaveBeenCalled();
  expect(sessionStorage.length).toBe(1);
});

it("can recover an existing intent after reload even if a slot is now occupied and fresh preflight expired", async () => {
  const saved = getRemoteMachineIntent("owner", "create", {
    free_slot: true,
    policy_version: "remote-machine-mvp",
    data_policy_version: "remote-machine-data",
    acknowledge_shared_environment: true,
    separation_choice: "automatic",
    separation_policy_version: "remote-machine-separation-v1",
    acknowledge_separation: true,
  });
  state.available = false;
  state.validUntil = "2020-01-01T00:00:00Z";
  show();
  await accept();
  await userEvent.click(screen.getByRole("button", { name: "Check status" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Retry saved create request" }),
    ).toBeEnabled(),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Retry saved create request" }),
  );
  await waitFor(() =>
    expect(state.create).toHaveBeenCalledWith({
      body: {
        ...saved.body,
        profile_version: "remote-machine",
        name: "my-remote-machine",
        capabilities: ["files", "shell"],
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
  expect(screen.queryByRole("form")).not.toBeInTheDocument();
  expect(screen.getByText("Occupied", { exact: true })).toBeVisible();
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
  expect(
    screen.getByRole("combobox", { name: "Machine owner" }),
  ).toBeDisabled();
  expect(screen.getByRole("checkbox", { name: /^Commands/ })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: /^Files/ })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: /^Browser/ })).not.toBeChecked();
  expect(screen.getByRole("checkbox", { name: /^Computer/ })).toBeEnabled();
  await userEvent.click(screen.getByRole("checkbox", { name: /^Computer/ }));
  expect(screen.getByRole("checkbox", { name: /^Browser/ })).toBeChecked();
  await userEvent.click(screen.getByRole("checkbox", { name: /^Browser/ }));
  expect(screen.getByRole("checkbox", { name: /^Computer/ })).not.toBeChecked();
  expect(screen.getByRole("checkbox", { name: /^Computer/ })).toBeEnabled();
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
    capabilities: ["browser", "computer", "files", "shell"],
    specialist_id: "specialist-1",
  });
  expect(screen.getByLabelText("Machine name")).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Review paid creation" }),
  ).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Check status" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Retry saved create request" }),
    ).toBeEnabled(),
  );
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
  if (
    screen
      .getByRole("checkbox", { name: /I accept the selected Agent separation/ })
      .getAttribute("data-state") !== "checked"
  )
    await userEvent.click(
      screen.getByRole("checkbox", {
        name: /I accept the selected Agent separation/,
      }),
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
    capabilities: ["browser", "files", "shell"],
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
  if (
    screen
      .getByRole("checkbox", { name: /I accept the selected Agent separation/ })
      .getAttribute("data-state") !== "checked"
  )
    await userEvent.click(
      screen.getByRole("checkbox", {
        name: /I accept the selected Agent separation/,
      }),
    );
  await userEvent.click(
    screen.getByRole("checkbox", { name: /I accept this exact rate/ }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Check status" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Retry saved create request" }),
    ).toBeEnabled(),
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
