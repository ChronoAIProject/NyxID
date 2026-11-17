import { StrictMode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi, type MockInstance } from "vitest";
import { router as appRouter } from "@/router";
import { useAuthStore } from "@/stores/auth-store";
import {
  platformSetup,
  setupActor,
  setupBinding,
  setupTemplates,
} from "@/test/platform-setup-fixture";

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
    drafts: [platformSetup()],
    saved: [] as ReturnType<typeof platformSetup>[],
    adoptionGate: Promise.resolve(),
    adoptionError: false,
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
      if (path === "/onboarding/setups") await setupsGate;
      const draft =
        fixture.drafts.find(
          (row) => path === `/onboarding/setups/${row.id}/adopt`,
        ) ??
        fixture.saved.find(
          (row) => path === `/onboarding/setups/${row.id}/adopt`,
        );
      if (draft) {
        fixture.adoptionBodies.push(JSON.parse(init!.body as string));
        await fixture.adoptionGate;
        if (fixture.adoptionError)
          return new Response(
            JSON.stringify({
              error: "conflict",
              message: "Draft changed. Review it again.",
            }),
            { status: 409 },
          );
        if (!fixture.saved.some((row) => row.id === draft.id))
          fixture.saved.push(draft);
        fixture.drafts = fixture.drafts.filter((row) => row.id !== draft.id);
        return Response.json(draft);
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
}, 20_000);

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
}, 20_000);

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

it("keeps the adoption error and reviewed revision available for one explicit retry", async () => {
  const { fixture } = await openSetups();
  fixture.adoptionError = true;
  const user = await reviewDraft();
  await user.dblClick(adoptionButton());
  await screen.findByText("Draft changed. Review it again.");
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
