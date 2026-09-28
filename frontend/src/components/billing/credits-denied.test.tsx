import {
  act,
  render,
  renderHook,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useVerifyChannelBot } from "@/hooks/use-channel-bots";
import type { CreditsPayer } from "@/lib/credits-denial";
import { useAuthStore } from "@/stores/auth-store";
import { useCreditsDenialStore } from "@/stores/credits-denial-store";
import type { User } from "@/types/api";
import { CreditsDeniedHost } from "./credits-denied-host";

const person: User = {
  id: "person-a",
  email: "a@example.com",
  display_name: "A",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: false,
  is_active: true,
  created_at: "2026-09-01T00:00:00Z",
  capabilities: { billing_available: true },
};

let client: QueryClient;

function renderAt(url: string) {
  const root = createRootRoute({
    component: () => (
      <>
        <Outlet />
        <CreditsDeniedHost />
      </>
    ),
  });
  const page = (path: string) =>
    createRoute({
      getParentRoute: () => root,
      path,
      component: () => <p>{path}</p>,
    });
  const history = createMemoryHistory({ initialEntries: [url] });
  const router = createRouter({
    routeTree: root.addChildren([
      page("/assistant"),
      page("/billing"),
      page("/login"),
    ]),
    history,
  });
  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return { history, router };
}

function deny(payer: CreditsPayer, key = "op:test:1") {
  act(() =>
    useCreditsDenialStore
      .getState()
      .notify({ key, payer, actorId: "person-a" }),
  );
}

beforeEach(() => {
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  useAuthStore.getState().setUser(person);
  useCreditsDenialStore.getState().reset();
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(JSON.stringify({ orgs: [] }))),
  );
});
afterEach(() => {
  useAuthStore.getState().setUser(null);
  vi.unstubAllGlobals();
});

describe("CreditsDeniedHost", () => {
  it("presents on /assistant, outside the dashboard layout, via a lazy dialog", async () => {
    renderAt("/assistant");
    await screen.findByText("/assistant");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    deny("self");
    expect(
      await screen.findByRole("dialog", {
        name: "Not enough credits to continue",
      }),
    ).toBeVisible();
    expect(
      screen.getByTestId("credits-denied-illustration"),
    ).toBeInTheDocument();
  });

  it.each(["/billing", "/login"])(
    "suppresses %s and drops the prompt",
    async (path) => {
      renderAt(path);
      await screen.findByText(path);
      deny("self");
      await waitFor(() =>
        expect(useCreditsDenialStore.getState().current).toBeNull(),
      );
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    },
  );

  it("renders nothing without a signed-in person", async () => {
    renderAt("/assistant");
    await screen.findByText("/assistant");
    act(() => {
      useCreditsDenialStore.setState({
        current: { key: "k", payer: "self", actorId: "person-a" },
      });
      useAuthStore.setState({ user: null, isAuthenticated: false });
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});

describe("CreditsDeniedDialog variants", () => {
  it("offers both options and the purchase CTA to the person paying", async () => {
    const { history } = renderAt("/assistant");
    deny("self");
    await screen.findByRole("dialog");
    expect(screen.getByText("Ask an admin")).toBeVisible();
    expect(screen.getByText("Purchase platform credits")).toBeVisible();
    expect(
      screen.queryByText(/If this is billed to an organization/),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Purchase credits" }),
    );
    await waitFor(() => expect(history.location.pathname).toBe("/billing"));
    const search = new URLSearchParams(history.location.search);
    expect(search.get("tab")).toBe("billing");
    expect(search.get("action")).toBe("topup");
    expect(useCreditsDenialStore.getState().current).toBeNull();
  });

  it("adds the organization hint when the payer is unknown", async () => {
    renderAt("/assistant");
    deny("unknown");
    await screen.findByRole("dialog");
    expect(
      screen.getByText(
        "If this is billed to an organization, ask one of its admins instead.",
      ),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Purchase credits" }),
    ).toBeVisible();
  });

  it("names the organization and never offers a purchase for org wallets", async () => {
    renderAt("/assistant");
    deny({ org: { id: "org-1", name: "Acme" } });
    await screen.findByRole("dialog");
    expect(screen.getByText(/billed to Acme's credits/)).toBeVisible();
    expect(
      screen.getByText("Only an admin of Acme can add credits to it."),
    ).toBeVisible();
    expect(
      screen.queryByText("Purchase platform credits"),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Got it" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });

  it("only suggests asking an admin when billing is unavailable", async () => {
    useAuthStore.getState().setUser({ ...person, capabilities: {} });
    renderAt("/assistant");
    deny("self");
    await screen.findByRole("dialog");
    expect(
      screen.queryByText("Purchase platform credits"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Purchase credits" }),
    ).toBeNull();
    expect(screen.getByRole("button", { name: "Got it" })).toBeVisible();
  });
});

describe("channel bot payer", () => {
  it("attributes an org-owned bot's denial to the org, without a purchase", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) =>
        String(url).endsWith("/verify")
          ? new Response(
              JSON.stringify({
                error: "insufficient_credits",
                error_code: 11300,
                message: "Insufficient credits",
              }),
              { status: 402 },
            )
          : new Response(JSON.stringify({ orgs: [] })),
      ),
    );
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
    const { result } = renderHook(
      () => useVerifyChannelBot({ creditsOwnerId: "org-1" }),
      { wrapper },
    );
    await act(async () => {
      await result.current.mutateAsync("bot-1").catch(() => undefined);
    });
    expect(useCreditsDenialStore.getState().current).toMatchObject({
      payer: { org: { id: "org-1" } },
      key: expect.stringMatching(/^op:channel-bot-verify:bot-1:/),
    });
    renderAt("/assistant");
    await screen.findByRole("dialog");
    expect(
      screen.queryByText("Purchase platform credits"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(/billed to your organization's credits/),
    ).toBeVisible();
  });
});
