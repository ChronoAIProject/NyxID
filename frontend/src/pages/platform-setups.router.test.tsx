import { StrictMode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  afterEach,
  beforeEach,
  expect,
  it,
  vi,
  type MockInstance,
} from "vitest";
import { router as appRouter } from "@/router";
import { useAuthStore } from "@/stores/auth-store";
import {
  platformSetup,
  setupActor,
  setupBinding,
  setupTemplates,
} from "@/test/platform-setup-fixture";

// Route loading may wait 10 seconds; give every case the same outer deadline.
vi.setConfig({ testTimeout: 20_000 });

const title = "Bring a Telegram draft into your account";
const initialAuth = useAuthStore.getState();
let client: QueryClient;
let consoleError: MockInstance<typeof console.error>;
beforeEach(() => {
  // React key warnings are console errors, not page errors. Fail even when
  // reconciliation happens to leave only one visible module in this run.
  consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
});
afterEach(() => {
  try {
    cleanup();
    client?.clear();
    useAuthStore.setState(initialAuth);
    expect(consoleError).not.toHaveBeenCalled();
  } finally {
    consoleError.mockRestore();
    vi.unstubAllGlobals();
  }
});

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

async function openSetups(setupsGate = Promise.resolve()) {
  const fixture = {
    actor: { ...setupActor },
    bindings: [{ ...setupBinding }],
    writes: [] as string[],
    adoptionBodies: [] as unknown[],
    draftReads: [] as string[],
    draftGate: Promise.resolve(),
    drafts: [platformSetup()],
    saved: [] as ReturnType<typeof platformSetup>[],
    adoptionGate: Promise.resolve(),
    adoptionError: false,
    bindingsError: false,
    draftErrorStatus: 0,
    revocationBodies: [] as unknown[],
  };
  // Keep the actual route tree, lazy page, DashboardLayout, hooks and schemas.
  // A mocked Outlet would hide exactly the composition regression in #2437.
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string, init?: RequestInit) => {
      const path = new URL(input, "http://localhost").pathname.replace(
        "/api/v1",
        "",
      );
      if (init?.method && init.method !== "GET") fixture.writes.push(path);
      if (
        path === `/onboarding/bindings/${setupBinding.id}/revoke` &&
        init?.method === "POST"
      ) {
        fixture.revocationBodies.push(JSON.parse(init.body as string));
        fixture.bindings = [];
        return new Response(null, { status: 204 });
      }
      if (path === "/onboarding/bindings" && fixture.bindingsError) {
        return Response.json(
          { error: "unavailable", message: "Fixture bindings unavailable." },
          { status: 503 },
        );
      }
      if (path === "/onboarding/setups") await setupsGate;
      if (path === "/onboarding/anonymous-drafts") {
        fixture.draftReads.push(input);
        await fixture.draftGate;
        if (fixture.draftErrorStatus) {
          const status = fixture.draftErrorStatus;
          const error = status === 403
            ? { error: "forbidden", message: "Binding is unavailable" }
            : status === 404
              ? { error: "not_found", message: "Platform bot configuration not found" }
              : { error: "unavailable", message: "Service temporarily unavailable." };
          return Response.json(error, { status });
        }
      }
      const draft =
        fixture.drafts.find(
          (row) => path === `/onboarding/setups/${row.id}/adopt`,
        ) ??
        fixture.saved.find(
          (row) => path === `/onboarding/setups/${row.id}/adopt`,
        );
      if (draft) {
        const body = JSON.parse(init!.body as string);
        fixture.adoptionBodies.push(body);
        await fixture.adoptionGate;
        const binding = fixture.bindings.find(
          (row) => row.id === body.binding_id,
        );
        if (!binding || binding.generation !== body.binding_generation)
          return Response.json(
            { error: "forbidden", message: "Binding is unavailable" },
            { status: 403 },
          );
        const current = fixture.drafts.find((row) => row.id === draft.id);
        const saved = fixture.saved.find((row) => row.id === draft.id);
        if (!saved && (!current || current.revision !== body.revision))
          return Response.json(
            { error: "conflict", message: "Anonymous draft expired or changed" },
            { status: 409 },
          );
        if (fixture.adoptionError)
          return Response.json(
            { error: "unavailable", message: "Service temporarily unavailable." },
            { status: 503 },
          );
        const adopted = saved ?? current!;
        if (!saved) fixture.saved.push(adopted);
        fixture.drafts = fixture.drafts.filter((row) => row.id !== adopted.id);
        return Response.json(adopted);
      }
      const responses: Record<string, unknown> = {
        "/users/me": fixture.actor,
        "/nodes": { nodes: [] },
        "/onboarding/templates": setupTemplates,
        "/onboarding/setups": fixture.saved,
        "/onboarding/setups/archived": {
          setups: [],
          next_before_ms: null,
          next_before_id: null,
        },
        "/onboarding/bindings": fixture.bindings,
        "/onboarding/anonymous-drafts": fixture.drafts,
      };
      if (!(path in responses))
        throw new Error(`Unexpected fixture request: ${path}`);
      return new Response(JSON.stringify(responses[path]), {
        headers: { "Content-Type": "application/json" },
      });
    }),
  );
  useAuthStore.setState({
    user: fixture.actor,
    isLoading: false,
    isAuthenticated: true,
  });
  const history = createMemoryHistory({
    initialEntries: ["/onboarding/setups"],
  });
  const router = createRouter({ routeTree: appRouter.routeTree, history });
  client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 60_000 } },
  });
  render(
    <StrictMode>
      <QueryClientProvider client={client}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    </StrictMode>,
  );
  await act(() => router.load());
  await act(() => vi.dynamicImportSettled());
  await screen.findByRole("heading", { name: title }, { timeout: 10_000 });
  return { fixture, history };
}

const adoptionButton = () =>
  screen.getByRole("button", { name: "Adopt reviewed draft" });

async function reviewDraft() {
  const user = userEvent.setup();
  await user.selectOptions(
    screen.getByLabelText("Linked chat"),
    setupBinding.id,
  );
  await user.click(await screen.findByRole("checkbox"));
  expect(adoptionButton()).toBeEnabled();
  return user;
}

async function expectSettled() {
  await waitFor(() => expect(client.isMutating()).toBe(0));
}

function expectSingleModule() {
  expect(screen.getAllByRole("main", { hidden: true })).toHaveLength(1);
  expect(
    screen.getAllByRole("heading", {
      name: "Your bot setups",
      hidden: true,
    }),
  ).toHaveLength(1);
  expect(
    screen.getAllByRole("heading", { name: title, hidden: true }),
  ).toHaveLength(1);
  expect(
    screen.getAllByRole("heading", {
      name: "Cancelled and expired setups",
      hidden: true,
    }),
  ).toHaveLength(1);
  expect(
    screen.getAllByRole("combobox", {
      name: "Linked chat",
      hidden: true,
    }),
  ).toHaveLength(1);
}

it("mounts one draft import page through direct entry, links and back/forward", async () => {
  const { history, fixture } = await openSetups();
  expectSingleModule();
  const user = userEvent.setup();
  await user.selectOptions(
    screen.getByLabelText("Linked chat"),
    setupBinding.id,
  );
  expect(
    await screen.findAllByRole("button", {
      name: "Adopt reviewed draft",
    }),
  ).toHaveLength(1);
  expect(screen.getAllByRole("checkbox")).toHaveLength(1);

  await user.click(screen.getByRole("link", { name: "Choose a starter" }));
  await screen.findByRole("heading", {
    name: "Your own Telegram assistant",
  });
  expect(
    screen.queryByRole("heading", { name: title }),
  ).not.toBeInTheDocument();
  await user.click(
    screen.getByRole("link", { name: "Resume your saved setups" }),
  );
  await screen.findByRole("heading", { name: title });
  expectSingleModule();
  await act(() => history.back());
  await screen.findByRole("heading", {
    name: "Your own Telegram assistant",
  });
  await act(() => history.forward());
  await screen.findByRole("heading", { name: title });
  expectSingleModule();
  expect(screen.getByLabelText("Linked chat")).toHaveValue("");
  expect(
    screen.queryByRole("button", { name: "Adopt reviewed draft" }),
  ).not.toBeInTheDocument();
  expect(fixture.writes).toEqual([]);
});

it("resets reviewed adoption on a binding generation or account change without duplicating controls", async () => {
  const { fixture } = await openSetups();
  const user = userEvent.setup();
  await user.selectOptions(
    screen.getByLabelText("Linked chat"),
    setupBinding.id,
  );
  await user.click(await screen.findByRole("checkbox"));
  expect(
    screen.getByRole("button", { name: "Adopt reviewed draft" }),
  ).toBeEnabled();
  fixture.bindings = [{ ...setupBinding, generation: 8 }];
  await act(() =>
    client.invalidateQueries({
      queryKey: ["onboarding", setupActor.id, "bindings"],
    }),
  );
  await waitFor(() => expect(screen.getByRole("checkbox")).not.toBeChecked());
  expect(
    screen.getByRole("button", { name: "Adopt reviewed draft" }),
  ).toBeDisabled();
  expectSingleModule();

  await user.click(screen.getByRole("checkbox"));
  fixture.actor = { ...setupActor, id: "66666666-6666-4666-8666-666666666666" };
  fixture.bindings = [];
  act(() => useAuthStore.getState().setUser(fixture.actor));
  expectSingleModule();
  expect(screen.getByLabelText("Linked chat")).toHaveValue("");
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Adopt reviewed draft" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText("Telegram cafe · revision 3"),
  ).not.toBeInTheDocument();
  expect(fixture.writes).toEqual([]);
});

it("keeps one module when the saved-setups query inserts its asynchronous empty state", async () => {
  const gate = deferred();
  try {
    await openSetups(gate.promise);
    expectSingleModule();
    expect(screen.getByText("Loading saved setups…")).toBeInTheDocument();
    await act(() => gate.resolve());
    await screen.findByText(
      "Your saved setups will appear here. Choose a starter to begin.",
    );
    expectSingleModule();
  } finally {
    gate.resolve();
  }
});

it("clears a revoked chat selection and requires reselecting a binding that reappears", async () => {
  const { fixture } = await openSetups();
  const user = await reviewDraft();
  await user.click(screen.getByRole("button", { name: "Review revocation" }));
  await user.click(
    screen.getByRole("button", { name: "Revoke binding" }),
  );
  await screen.findByText(
    "No active onboarding bindings. Use /help in the manager’s private chat to start onboarding.",
  );
  await expectSettled();
  expect(screen.getByLabelText("Linked chat")).toHaveValue("");
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Adopt reviewed draft" }),
  ).not.toBeInTheDocument();
  expect(fixture.writes).toEqual([
    `/onboarding/bindings/${setupBinding.id}/revoke`,
  ]);
  expect(fixture.revocationBodies).toEqual([
    { generation: setupBinding.generation },
  ]);
  expectSingleModule();

  fixture.bindings = [{ ...setupBinding }];
  await act(() =>
    client.invalidateQueries({
      queryKey: ["onboarding", setupActor.id, "bindings"],
    }),
  );
  await screen.findByRole("option", { name: /Telegram sender/ });
  expect(screen.getByLabelText("Linked chat")).toHaveValue("");
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Adopt reviewed draft" }),
  ).not.toBeInTheDocument();
  expect(fixture.writes).toHaveLength(1);
  expectSingleModule();

  await user.selectOptions(
    screen.getByLabelText("Linked chat"),
    setupBinding.id,
  );
  expect(await screen.findByRole("checkbox")).not.toBeChecked();
  expect(adoptionButton()).toBeDisabled();
});

it("retains the selected chat and review when a bindings refetch fails", async () => {
  const { fixture } = await openSetups();
  await reviewDraft();
  fixture.bindingsError = true;
  await act(() =>
    client.invalidateQueries({
      queryKey: ["onboarding", setupActor.id, "bindings"],
    }),
  );
  await screen.findByText("Could not load your bindings.");
  expect(screen.getByLabelText("Linked chat")).toHaveValue(setupBinding.id);
  expect(screen.getByRole("checkbox")).toBeChecked();
  expect(adoptionButton()).toBeEnabled();
  expect(fixture.writes).toEqual([]);
  expectSingleModule();
});

it("settles one rapid adoption, refreshes to empty and requires review for the next draft", async () => {
  const { fixture } = await openSetups();
  const gate = deferred();
  fixture.adoptionGate = gate.promise;
  const user = await reviewDraft();
  try {
    await user.dblClick(adoptionButton());
    await waitFor(() => expect(fixture.writes).toHaveLength(1));
    expect(adoptionButton()).toBeDisabled();
    expect(fixture.adoptionBodies).toEqual([
      {
        revision: 3,
        binding_id: setupBinding.id,
        binding_generation: 7,
      },
    ]);
  } finally {
    await act(() => gate.resolve());
  }
  await screen.findByText("No unowned drafts remain for this chat.");
  await expectSettled();
  expect(fixture.saved).toHaveLength(1);
  expect(fixture.writes).toHaveLength(1);
  expectSingleModule();

  fixture.drafts = [
    platformSetup({ id: "55555555-5555-4555-8555-555555555555" }),
  ];
  await act(() =>
    client.invalidateQueries({
      queryKey: ["onboarding", setupActor.id, "anonymous"],
    }),
  );
  expect(await screen.findByRole("checkbox")).not.toBeChecked();
  expect(adoptionButton()).toBeDisabled();
  await user.click(screen.getByRole("checkbox"));
  await user.dblClick(adoptionButton());
  await screen.findByText("No unowned drafts remain for this chat.");
  await expectSettled();
  expect(fixture.writes).toHaveLength(2);
  expect(fixture.saved).toHaveLength(2);
});

it("retains the reviewed revision for one explicit retry after a transient failure", async () => {
  const { fixture } = await openSetups();
  fixture.adoptionError = true;
  const user = await reviewDraft();
  await user.dblClick(adoptionButton());
  await screen.findByText("Service temporarily unavailable.");
  await expectSettled();
  expect(adoptionButton()).toBeEnabled();
  expect(screen.getByRole("checkbox")).toBeChecked();
  expect(fixture.writes).toHaveLength(1);
  fixture.adoptionError = false;
  await user.dblClick(adoptionButton());
  await screen.findByText("No unowned drafts remain for this chat.");
  await expectSettled();
  expect(fixture.writes).toHaveLength(2);
  expect(fixture.adoptionBodies[1]).toEqual(fixture.adoptionBodies[0]);
});

it("releases the click guard when hook-level success invalidation rejects", async () => {
  const { fixture } = await openSetups();
  const user = await reviewDraft();
  const invalidation = vi
    .spyOn(client, "invalidateQueries")
    .mockRejectedValueOnce(new Error("Fixture refresh failed."));
  try {
    await user.click(adoptionButton());
    await screen.findByText("Fixture refresh failed.");
    await expectSettled();
    expect(adoptionButton()).toBeEnabled();
    expect(fixture.writes).toHaveLength(1);
    await user.dblClick(adoptionButton());
    await screen.findByText("No unowned drafts remain for this chat.");
    await expectSettled();
    expect(fixture.writes).toHaveLength(2);
    expect(fixture.adoptionBodies[1]).toEqual(fixture.adoptionBodies[0]);
    expect(fixture.saved).toHaveLength(1);
  } finally {
    invalidation.mockRestore();
  }
});

for (const reset of ["binding generation", "account"] as const) {
  it(`discards consent on an in-flight ${reset} remount and ignores the old observer's success callback`, async () => {
    const { fixture } = await openSetups();
    const gate = deferred();
    fixture.adoptionGate = gate.promise;
    const user = await reviewDraft();
    await user.click(adoptionButton());
    await waitFor(() => expect(fixture.writes).toHaveLength(1));
    try {
      fixture.bindings = [{ ...setupBinding, generation: 8 }];
      if (reset === "account") {
        fixture.actor = {
          ...setupActor,
          id: "66666666-6666-4666-8666-666666666666",
        };
        act(() => useAuthStore.getState().setUser(fixture.actor));
        expect(screen.getByLabelText("Linked chat")).toHaveValue("");
        await screen.findByRole("option", { name: /Telegram sender/ });
        await user.selectOptions(
          screen.getByLabelText("Linked chat"),
          setupBinding.id,
        );
      } else {
        await act(() =>
          client.invalidateQueries({
            queryKey: ["onboarding", setupActor.id, "bindings"],
          }),
        );
      }
      await waitFor(() =>
        expect(screen.getByRole("checkbox")).not.toBeChecked(),
      );
      expect(adoptionButton()).toBeDisabled();
      expect(fixture.writes).toHaveLength(1);
      expectSingleModule();

      // A different unowned draft is reviewed in the new instance while the
      // first request is still pending. Its consent must survive that reply.
      fixture.drafts = [
        platformSetup({
          id: "55555555-5555-4555-8555-555555555555",
          facts: { ...platformSetup().facts, name: "Second Telegram draft" },
        }),
      ];
      await act(() =>
        client.invalidateQueries({
          queryKey: ["onboarding", fixture.actor.id, "anonymous"],
        }),
      );
      await screen.findByText("Second Telegram draft · revision 3");
      await user.click(screen.getByRole("checkbox"));
      await act(() => gate.resolve());
      await expectSettled();
      expect(screen.getByRole("checkbox")).toBeChecked();
      expect(adoptionButton()).toBeEnabled();
      expect(fixture.writes).toHaveLength(1);
      await user.dblClick(adoptionButton());
      await screen.findByText("No unowned drafts remain for this chat.");
      await expectSettled();
      expect(fixture.writes).toHaveLength(2);
      expect(fixture.adoptionBodies).toEqual([
        { revision: 3, binding_id: setupBinding.id, binding_generation: 7 },
        { revision: 3, binding_id: setupBinding.id, binding_generation: 8 },
      ]);
    } finally {
      await act(() => gate.resolve());
    }
  });
}

it("requires a fresh explicit review to resubmit the same draft after a pending review is remounted", async () => {
  const { fixture } = await openSetups();
  const gate = deferred();
  fixture.adoptionGate = gate.promise;
  const user = await reviewDraft();
  try {
    await user.click(adoptionButton());
    await waitFor(() => expect(fixture.writes).toHaveLength(1));
    await user.selectOptions(screen.getByLabelText("Linked chat"), "");
    await user.selectOptions(
      screen.getByLabelText("Linked chat"),
      setupBinding.id,
    );
    expect(await screen.findByRole("checkbox")).not.toBeChecked();
    expect(adoptionButton()).toBeDisabled();
    expect(fixture.writes).toHaveLength(1);

    // This is a new, intentional request. The server makes same-owner,
    // same-binding adoption idempotent; the UI does not retain old consent.
    await user.click(screen.getByRole("checkbox"));
    await user.dblClick(adoptionButton());
    await waitFor(() => expect(fixture.writes).toHaveLength(2));
    expect(fixture.adoptionBodies[1]).toEqual(fixture.adoptionBodies[0]);
  } finally {
    await act(() => gate.resolve());
    await expectSettled();
  }
  await screen.findByText("No unowned drafts remain for this chat.");
  expect(fixture.saved).toHaveLength(1);
});

it("refetches a rejected revision and requires fresh review before adopting it", async () => {
  const { fixture } = await openSetups();
  const user = await reviewDraft();
  fixture.drafts = [platformSetup({ revision: 4 })];
  await user.click(adoptionButton());
  await screen.findByText("Anonymous draft expired or changed");
  await expectSettled();
  await screen.findByText("Telegram cafe · revision 4");
  expect(screen.getByRole("checkbox")).not.toBeChecked();
  expect(adoptionButton()).toBeDisabled();
  expect(fixture.adoptionBodies).toEqual([
    {
      revision: 3,
      binding_id: setupBinding.id,
      binding_generation: setupBinding.generation,
    },
  ]);
  expect(fixture.draftReads).toHaveLength(2);
  expect(
    new URL(fixture.draftReads[1]!, "http://localhost").searchParams.get(
      "binding_id",
    ),
  ).toBe(setupBinding.id);
  expect(fixture.saved).toHaveLength(0);
  expectSingleModule();
  await user.click(screen.getByRole("checkbox"));
  await user.click(adoptionButton());
  await screen.findByText("No unowned drafts remain for this chat.");
  await expectSettled();
  expect(fixture.adoptionBodies[1]).toEqual({
    revision: 4,
    binding_id: setupBinding.id,
    binding_generation: setupBinding.generation,
  });
  expect(fixture.writes).toHaveLength(2);
  expect(fixture.saved).toHaveLength(1);
});

it("preserves the newer review of B when pending adoption of A succeeds", async () => {
  const { fixture } = await openSetups();
  const second = platformSetup({
    id: "55555555-5555-4555-8555-555555555555",
    facts: { ...platformSetup().facts, name: "Second draft" },
  });
  fixture.drafts.push(second);
  const user = userEvent.setup();
  await user.selectOptions(
    screen.getByLabelText("Linked chat"),
    setupBinding.id,
  );
  const boxes = await screen.findAllByRole("checkbox");
  expect(boxes).toHaveLength(2);
  const gate = deferred();
  fixture.adoptionGate = gate.promise;
  try {
    await user.click(boxes[0]!);
    await user.click(
      screen.getAllByRole("button", { name: "Adopt reviewed draft" })[0]!,
    );
    await waitFor(() => expect(fixture.writes).toHaveLength(1));
    await user.click(boxes[1]!);
    expect(boxes[1]).toBeChecked();
    expect(
      screen.getAllByRole("button", { name: "Adopt reviewed draft" })[1]!,
    ).toBeDisabled();
  } finally {
    await act(() => gate.resolve());
  }
  await expectSettled();
  await waitFor(() => expect(screen.getAllByRole("checkbox")).toHaveLength(1));
  expect(screen.getByRole("checkbox")).toBeChecked();
  expect(adoptionButton()).toBeEnabled();
  expect(fixture.drafts).toEqual([second]);
  expect(fixture.saved).toHaveLength(1);
  expect(fixture.writes).toEqual([
    `/onboarding/setups/${platformSetup().id}/adopt`,
  ]);
  await user.click(adoptionButton());
  await screen.findByText("No unowned drafts remain for this chat.");
  await expectSettled();
  expect(fixture.writes).toEqual([
    `/onboarding/setups/${platformSetup().id}/adopt`,
    `/onboarding/setups/${second.id}/adopt`,
  ]);
  expect(fixture.saved).toHaveLength(2);
  expectSingleModule();
});

it.each([403, 404])(
  "discards private facts on draft read denial %s until an authorized retry",
  async (status) => {
    const { fixture } = await openSetups();
    const user = await reviewDraft();
    const queryKey = [
      "onboarding",
      setupActor.id,
      "anonymous",
      setupBinding.id,
      setupBinding.generation,
    ];
    fixture.draftErrorStatus = status;
    await act(() => client.invalidateQueries({ queryKey, exact: true }));
    await screen.findByText(
      "Could not load drafts for this chat. Refresh your binding before trying again.",
    );
    const expectNoPrivateDraft = () => {
      expect(
        screen.queryByText("Telegram cafe · revision 3"),
      ).not.toBeInTheDocument();
      expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Adopt reviewed draft" }),
      ).not.toBeInTheDocument();
      const cached = JSON.stringify(client.getQueryData(queryKey));
      expect(cached).not.toContain('"facts"');
      expect(cached).not.toContain("Telegram cafe");
      expect(fixture.writes).toEqual([]);
      expect(screen.getByLabelText("Linked chat")).toHaveValue(setupBinding.id);
      expectSingleModule();
    };
    expectNoPrivateDraft();
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
    fixture.draftErrorStatus = 503;
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() =>
      expect(client.getQueryState(queryKey)?.status).toBe("error"),
    );
    expectNoPrivateDraft();
    await waitFor(
      () => expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled(),
      { timeout: 3_000 },
    );
    fixture.draftErrorStatus = 0;
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByText("Telegram cafe · revision 3");
    expect(screen.getByRole("checkbox")).not.toBeChecked();
    expect(adoptionButton()).toBeDisabled();
    expect(fixture.writes).toEqual([]);
    await user.click(screen.getByRole("checkbox"));
    expect(adoptionButton()).toBeEnabled();
  },
);

it("preserves review B when pending adoption A gets a definitive refusal", async () => {
  const { fixture } = await openSetups();
  const second = platformSetup({
    id: "55555555-5555-4555-8555-555555555555",
    facts: { ...platformSetup().facts, name: "Second draft" },
  });
  fixture.drafts.push(second);
  const user = userEvent.setup();
  await user.selectOptions(
    screen.getByLabelText("Linked chat"),
    setupBinding.id,
  );
  const boxes = await screen.findAllByRole("checkbox");
  expect(boxes).toHaveLength(2);
  const gate = deferred();
  fixture.adoptionGate = gate.promise;
  try {
    await user.click(boxes[0]!);
    await user.click(
      screen.getAllByRole("button", { name: "Adopt reviewed draft" })[0]!,
    );
    await waitFor(() => expect(fixture.writes).toHaveLength(1));
    fixture.drafts = [platformSetup({ revision: 4 }), second];
    await user.click(boxes[1]!);
    expect(boxes[1]).toBeChecked();
  } finally {
    await act(() => gate.resolve());
  }
  await screen.findByText("Anonymous draft expired or changed");
  await expectSettled();
  expect(screen.getAllByRole("checkbox")[0]).not.toBeChecked();
  expect(screen.getAllByRole("checkbox")[1]).toBeChecked();
  expect(
    screen.getAllByRole("button", { name: "Adopt reviewed draft" })[1]!,
  ).toBeEnabled();
  expect(fixture.writes).toHaveLength(1);
  expect(fixture.saved).toHaveLength(0);
  await user.click(
    screen.getAllByRole("button", { name: "Adopt reviewed draft" })[1]!,
  );
  await expectSettled();
  await waitFor(() => expect(fixture.saved).toEqual([second]));
  expect(fixture.writes).toHaveLength(2);
});

it("retains authorized facts and consent after a transient draft read failure", async () => {
  const { fixture } = await openSetups();
  await reviewDraft();
  fixture.draftErrorStatus = 503;
  await act(() =>
    client.invalidateQueries({
      queryKey: [
        "onboarding",
        setupActor.id,
        "anonymous",
        setupBinding.id,
        setupBinding.generation,
      ],
      exact: true,
    }),
  );
  await screen.findByText(
    "Could not load drafts for this chat. Refresh your binding before trying again.",
  );
  expect(screen.getByText("Telegram cafe · revision 3")).toBeVisible();
  expect(screen.getByRole("checkbox")).toBeChecked();
  expect(adoptionButton()).toBeEnabled();
  expect(fixture.writes).toEqual([]);
});

it("invalidates only the submitted chat after refusal without an observer", async () => {
  const { fixture } = await openSetups();
  const user = await reviewDraft();
  const gate = deferred();
  fixture.adoptionGate = gate.promise;
  fixture.drafts = [platformSetup({ revision: 4 })];
  const invalidation = vi.spyOn(client, "invalidateQueries");
  try {
    await user.click(adoptionButton());
    await waitFor(() => expect(fixture.writes).toHaveLength(1));
    await user.selectOptions(screen.getByLabelText("Linked chat"), "");
    await act(() => gate.resolve());
    await expectSettled();
    expect(invalidation).toHaveBeenCalledExactlyOnceWith({
      queryKey: [
        "onboarding",
        setupActor.id,
        "anonymous",
        setupBinding.id,
        setupBinding.generation,
      ],
      exact: true,
    });
    expect(
      client.getQueryState(["onboarding", setupActor.id, "setups"])
        ?.isInvalidated,
    ).toBe(false);
    expect(fixture.writes).toHaveLength(1);
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    expectSingleModule();
  } finally {
    await act(() => gate.resolve());
    await expectSettled();
    invalidation.mockRestore();
  }
});

it("preserves the refusal and releases the guard if its exact-query refresh rejects", async () => {
  const { fixture } = await openSetups();
  const user = await reviewDraft();
  fixture.drafts = [platformSetup({ revision: 4 })];
  const invalidation = vi
    .spyOn(client, "invalidateQueries")
    .mockRejectedValueOnce(new Error("Fixture refresh failed."));
  try {
    await user.click(adoptionButton());
    await screen.findByText("Anonymous draft expired or changed");
    await expectSettled();
    expect(
      screen.queryByText("Telegram cafe · revision 3"),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Adopt reviewed draft" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("Fixture refresh failed."),
    ).not.toBeInTheDocument();
    expect(fixture.writes).toHaveLength(1);
    await act(() =>
      client.invalidateQueries({
        queryKey: [
          "onboarding",
          setupActor.id,
          "anonymous",
          setupBinding.id,
          setupBinding.generation,
        ],
        exact: true,
      }),
    );
    await screen.findByText("Telegram cafe · revision 4");
    await user.click(screen.getByRole("checkbox"));
    await user.click(adoptionButton());
    await screen.findByText("No unowned drafts remain for this chat.");
    await expectSettled();
    expect(fixture.writes).toHaveLength(2);
    expect(fixture.adoptionBodies[1]).toEqual({
      revision: 4,
      binding_id: setupBinding.id,
      binding_generation: setupBinding.generation,
    });
  } finally {
    invalidation.mockRestore();
  }
});

it.each([0, 503])(
  "keeps an authoritative empty list visible during a background read returning %s",
  async (status) => {
    const { fixture } = await openSetups();
    fixture.drafts = [];
    const user = userEvent.setup();
    await user.selectOptions(
      screen.getByLabelText("Linked chat"),
      setupBinding.id,
    );
    const empty = "No unowned drafts remain for this chat.";
    await screen.findByText(empty);
    const queryKey = [
      "onboarding",
      setupActor.id,
      "anonymous",
      setupBinding.id,
      setupBinding.generation,
    ];
    const gate = deferred();
    fixture.draftGate = gate.promise;
    fixture.draftErrorStatus = status;
    let refresh: Promise<void> | undefined;
    try {
      await act(async () => {
        refresh = client.invalidateQueries({ queryKey, exact: true });
        // Flush the observer notification while the background GET stays held.
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
      expect(client.getQueryState(queryKey)?.fetchStatus).toBe("fetching");
      expect(fixture.draftReads).toHaveLength(2);
      expect(screen.getByText(empty)).toBeVisible();
    } finally {
      await act(async () => {
        gate.resolve();
        await refresh;
      });
    }
    if (status) {
      expect(await screen.findByRole("button", { name: "Retry" })).toBeVisible();
    }
    expect(screen.getByText(empty)).toBeVisible();
    expect(client.getQueryState(queryKey)?.status).toBe(status ? "error" : "success");
    expect(client.getQueryData(queryKey)).toEqual({ status: "ready", drafts: [] });
    expect(fixture.writes).toHaveLength(0);
    expectSingleModule();
  },
);

it("shows an authoritative empty list after a provisional recovery read succeeds", async () => {
  const { fixture } = await openSetups();
  const user = await reviewDraft();
  fixture.drafts = [platformSetup({ revision: 4 })];
  const gate = deferred();
  fixture.draftGate = gate.promise;
  const queryKey = [
    "onboarding",
    setupActor.id,
    "anonymous",
    setupBinding.id,
    setupBinding.generation,
  ];
  try {
    await user.click(adoptionButton());
    await waitFor(() => expect(fixture.draftReads).toHaveLength(2));
    fixture.drafts = [];
    expect(client.getQueryState(queryKey)?.fetchStatus).toBe("fetching");
    expect(client.getQueryData(queryKey)).toEqual({
      status: "ready",
      drafts: [],
      provisional: true,
    });
    expect(
      screen.queryByText("No unowned drafts remain for this chat."),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  } finally {
    await act(() => gate.resolve());
  }
  await screen.findByText("No unowned drafts remain for this chat.");
  await expectSettled();
  expect(client.getQueryData(queryKey)).toEqual({ status: "ready", drafts: [] });
  expect(fixture.writes).toHaveLength(1);
  expect(fixture.saved).toHaveLength(0);
  expectSingleModule();
});

it("removes the rejected snapshot through failed refreshes until a new read succeeds", async () => {
  const { fixture } = await openSetups();
  const user = await reviewDraft();
  const gate = deferred();
  const queryKey = [
    "onboarding",
    setupActor.id,
    "anonymous",
    setupBinding.id,
    setupBinding.generation,
  ];
  fixture.adoptionGate = gate.promise;
  try {
    await user.click(adoptionButton());
    await waitFor(() => expect(fixture.writes).toHaveLength(1));
    fixture.drafts = [platformSetup({ revision: 4 })];
    fixture.draftErrorStatus = 503;
  } finally {
    await act(() => gate.resolve());
  }
  await screen.findByText("Anonymous draft expired or changed");
  await expectSettled();
  const expectRejectedSnapshotGone = () => {
    expect(
      screen.queryByText("Telegram cafe · revision 3"),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Adopt reviewed draft" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("No unowned drafts remain for this chat."),
    ).not.toBeInTheDocument();
    expect(client.getQueryState(queryKey)?.status).toBe("error");
    expect(client.getQueryData(queryKey)).toEqual({
      status: "ready",
      drafts: [],
      provisional: true,
    });
    expect(fixture.adoptionBodies).toEqual([
      { revision: 3, binding_id: setupBinding.id, binding_generation: 7 },
    ]);
    expectSingleModule();
  };
  await screen.findByText(
    "Could not load drafts for this chat. Refresh your binding before trying again.",
  );
  expectRejectedSnapshotGone();
  await user.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(fixture.draftReads).toHaveLength(3));
  await waitFor(() =>
    expect(client.isFetching({ queryKey, exact: true })).toBe(0),
  );
  expectRejectedSnapshotGone();
  await waitFor(
    () => expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled(),
    { timeout: 3_000 },
  );
  fixture.draftErrorStatus = 0;
  await user.click(screen.getByRole("button", { name: "Retry" }));
  await screen.findByText("Telegram cafe · revision 4");
  expect(screen.getByRole("checkbox")).not.toBeChecked();
  expect(adoptionButton()).toBeDisabled();
  expect(fixture.writes).toHaveLength(1);
  await user.click(screen.getByRole("checkbox"));
  await user.click(adoptionButton());
  await screen.findByText("No unowned drafts remain for this chat.");
  await expectSettled();
  expect(fixture.adoptionBodies).toEqual([
    { revision: 3, binding_id: setupBinding.id, binding_generation: 7 },
    { revision: 4, binding_id: setupBinding.id, binding_generation: 7 },
  ]);
  expect(fixture.saved.map((draft) => draft.revision)).toEqual([4]);
});

it("removes rejected A while preserving reviewed B after a failed conflict refresh", async () => {
  const { fixture } = await openSetups();
  const second = platformSetup({
    id: "55555555-5555-4555-8555-555555555555",
    facts: { ...platformSetup().facts, name: "Second draft" },
  });
  fixture.drafts.push(second);
  const user = userEvent.setup();
  await user.selectOptions(
    screen.getByLabelText("Linked chat"),
    setupBinding.id,
  );
  const boxes = await screen.findAllByRole("checkbox");
  expect(boxes).toHaveLength(2);
  const gate = deferred();
  fixture.adoptionGate = gate.promise;
  try {
    await user.click(boxes[0]!);
    await user.click(
      screen.getAllByRole("button", { name: "Adopt reviewed draft" })[0]!,
    );
    await waitFor(() => expect(fixture.writes).toHaveLength(1));
    fixture.drafts = [platformSetup({ revision: 4 }), second];
    fixture.draftErrorStatus = 503;
    await user.click(boxes[1]!);
  } finally {
    await act(() => gate.resolve());
  }
  await screen.findByText("Anonymous draft expired or changed");
  await expectSettled();
  expect(
    screen.queryByText("Telegram cafe · revision 3"),
  ).not.toBeInTheDocument();
  expect(screen.getAllByRole("checkbox")).toHaveLength(1);
  expect(screen.getByRole("checkbox")).toBeChecked();
  expect(adoptionButton()).toBeEnabled();
  expect(fixture.writes).toHaveLength(1);
  expect(fixture.saved).toHaveLength(0);
  await user.click(adoptionButton());
  await expectSettled();
  expect(fixture.writes).toEqual([
    `/onboarding/setups/${platformSetup().id}/adopt`,
    `/onboarding/setups/${second.id}/adopt`,
  ]);
  expect(fixture.saved).toEqual([second]);
  expectSingleModule();
});

it.each(["newer revision", "denied marker", "authoritative empty list"] as const)(
  "preserves a cached %s when an older adoption is refused",
  async (cached) => {
    const { fixture } = await openSetups();
    const user = await reviewDraft();
    const gate = deferred();
    const queryKey = [
      "onboarding",
      setupActor.id,
      "anonymous",
      setupBinding.id,
      setupBinding.generation,
    ];
    const data =
      cached === "newer revision"
        ? { status: "ready", drafts: [platformSetup({ revision: 4 })] }
        : cached === "denied marker"
          ? { status: "denied" }
          : { status: "ready", drafts: [] };
    fixture.adoptionGate = gate.promise;
    try {
      await user.click(adoptionButton());
      await waitFor(() => expect(fixture.writes).toHaveLength(1));
      fixture.drafts = [platformSetup({ revision: 4 })];
      fixture.draftErrorStatus = 503;
      act(() => client.setQueryData(queryKey, data));
    } finally {
      await act(() => gate.resolve());
    }
    await screen.findByText("Anonymous draft expired or changed");
    await expectSettled();
    expect(client.getQueryData(queryKey)).toEqual(data);
    expect(fixture.writes).toHaveLength(1);
    if (cached === "newer revision") {
      expect(screen.getByText("Telegram cafe · revision 4")).toBeVisible();
      expect(screen.getByRole("checkbox")).not.toBeChecked();
    } else {
      expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
      if (cached === "authoritative empty list") {
        expect(
          screen.getByText("No unowned drafts remain for this chat."),
        ).toBeVisible();
      }
    }
  },
);
