import { StrictMode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
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
afterEach(() => {
  cleanup();
  client?.clear();
  useAuthStore.setState(initialAuth);
  vi.unstubAllGlobals();
});

async function openSetups() {
  const fixture = {
    actor: { ...setupActor },
    bindings: [{ ...setupBinding }],
    writes: [] as string[],
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
      const responses: Record<string, unknown> = {
        "/users/me": fixture.actor,
        "/nodes": { nodes: [] },
        "/onboarding/templates": setupTemplates,
        "/onboarding/setups": [],
        "/onboarding/setups/archived": {
          setups: [],
          next_before_ms: null,
          next_before_id: null,
        },
        "/onboarding/bindings": fixture.bindings,
        "/onboarding/anonymous-drafts": [platformSetup()],
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
