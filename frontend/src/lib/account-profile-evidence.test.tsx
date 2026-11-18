import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { screen } from "@testing-library/react";
import { SettingsPage } from "@/pages/settings";
import { TooltipProvider } from "@/components/ui/tooltip";
import { test, expect, vi } from "vitest";
import { act, render, waitFor, cleanup } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { useUser } from "@/hooks/use-auth";
import { useAuthStore as auth } from "@/stores/auth-store";
import { createAccountQueryScope } from "./account-query-cache";
import { installAccountSessionSync } from "./account-session-sync";
import { withErasurePreparation } from "./erasure-preparation";
import { apiClient, ApiError } from "./api-client";
import { isAccountAbort } from "./assistant/identity";
import type { User } from "@/types/api";

const person = (id: string) => ({ id, email: id + "@example.test" }) as User;
const tick = () => new Promise((r) => setTimeout(r, 10));
function fixture() {
  auth.getState().setUser(person("A"));
  const published: string[] = [],
    posts: Array<string | null> = [],
    read: Array<(u: string | Response) => void> = [];
  let hint!: (id: string) => void;
  vi.stubGlobal(
    "BroadcastChannel",
    class {
      set onmessage(fn: (event: { data: { userId: string; nonce: string } }) => void) {
        hint = (userId) => fn({ data: { userId, nonce: crypto.randomUUID() } });
      }
      postMessage(m: { userId: string | null }) {
        posts.push(m.userId);
      }
      close() {}
    },
  );
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string, init: RequestInit) => {
      if (url !== "/api/v1/users/me" || init.method !== "GET")
        throw Error("unexpected network attempt");
      return new Promise<Response>((r) =>
        read.push((id) =>
          r(typeof id === "string" ? Response.json(person(id)) : id),
        ),
      );
    }),
  );
  const stop = installAccountSessionSync(),
    unsub = auth.subscribe((s) => published.push(s.user?.id ?? "signed-out"));
  return {
    hint,
    read,
    published,
    posts,
    dispose() {
      stop();
      unsub();
      cleanup();
      vi.unstubAllGlobals();
    },
  };
}
test("isolated verifier supersedes B, stays covered and broadcasts no B", async () => {
  const f = fixture();
  try {
    f.hint("B");
    await waitFor(() => expect(f.read).toHaveLength(1));
    f.hint("A");
    await waitFor(() => expect(f.read).toHaveLength(2));
    f.read[0]!("B");
    await tick();
    expect(auth.getState().user?.id).toBe("A");
    expect(
      document.documentElement.hasAttribute("data-account-verifying"),
    ).toBe(true);
    f.read[1]!("A");
    await tick();
    expect(auth.getState().user?.id).toBe("A");
    expect(f.posts).not.toContain("B");
    expect(f.published).not.toContain("B");
  } finally {
    f.dispose();
  }
});
test("a simultaneously mounted useUser read must not overwrite the latest confirmed A", async () => {
  const f = fixture(),
    scope = createAccountQueryScope({});
  scope.getClient().setQueryData(["user", "me"], person("A"));
  function View() {
    const p = useUser();
    return <p>{p.data?.email}</p>;
  }
  try {
    render(
      <QueryClientProvider client={scope.getClient()}>
        <View />
      </QueryClientProvider>,
    );
    f.hint("B");
    await waitFor(() => expect(f.read).toHaveLength(1));
    void scope.getClient().invalidateQueries({ queryKey: ["user", "me"] });
    await waitFor(() => expect(f.read).toHaveLength(2));
    f.hint("A");
    await waitFor(() => expect(f.read).toHaveLength(3));
    await act(async () => {
      f.read[0]!("B");
      f.read[2]!("A");
      await tick();
    });
    expect(auth.getState().user?.id).toBe("A");
    expect(
      document.documentElement.hasAttribute("data-account-verifying"),
    ).toBe(false);
    await act(async () => {
      f.read[1]!("B");
      await tick();
    });
    expect([auth.getState().user?.id, f.posts]).toEqual(["A", []]);
    expect(f.published).not.toContain("B");
    expect(f.published).not.toContain("signed-out");
    expect(scope.getClient().getQueryData<User>(["user", "me"])?.id).not.toBe("B");
  } finally {
    f.dispose();
    scope.dispose();
  }
});
test("500ms backoff cannot start a replacement-subject dispatch", async () => {
  const f = fixture();
  try {
    let owner = "A";
    const calls: string[] = [];
    const result = withErasurePreparation(async () => {
      calls.push(owner);
      throw new ApiError(409, {
        error: "erasure_preparing",
        error_code: 12801,
        message: "preparing",
      });
    }).catch((e) => e);
    owner = "B";
    f.hint("B");
    expect(isAccountAbort(await result)).toBe(true);
    expect(calls).toEqual(["A"]);
    f.read[0]!("B");
    await tick();
    expect(auth.getState().user?.id).toBe("B");
    expect(calls).toEqual(["A"]);
  } finally {
    f.dispose();
  }
});
test.each(["200", "401"])(
  "late checkAuth %s cannot replace the latest confirmed A",
  async (status) => {
    const f = fixture();
    try {
      f.hint("B");
      await waitFor(() => expect(f.read).toHaveLength(1));
      const independent = auth.getState().checkAuth();
      await waitFor(() => expect(f.read).toHaveLength(2));
      f.hint("A");
      await waitFor(() => expect(f.read).toHaveLength(3));
      f.read[0]!("B");
      f.read[2]!("A");
      await tick();
      f.read[1]!(status === "200" ? "B" : Response.json({}, { status: 401 }));
      await independent;
      await tick();
      expect([auth.getState().user?.id, f.posts]).toEqual(["A", []]);
    } finally {
      f.dispose();
    }
  },
);


test("same-account useUser refresh still updates capabilities and feature flags", async () => {
  const f = fixture();
  const scope = createAccountQueryScope({});
  function View() { useUser(); return null; }
  try {
    render(<QueryClientProvider client={scope.getClient()}><View /></QueryClientProvider>);
    await waitFor(() => expect(f.read).toHaveLength(1));
    const updated = { ...person("A"), is_admin: true, feature_flags: { assistant: true } };
    await act(async () => { f.read[0]!(Response.json(updated)); await tick(); });
    expect(auth.getState().user).toEqual(updated);
    expect(scope.getClient().getQueryData(["user", "me"])).toEqual(updated);
    expect(f.posts).toEqual([]);
  } finally { f.dispose(); scope.dispose(); }
});

test("a transport profile 401 from older evidence cannot sign out confirmed A", async () => {
  const f = fixture();
  try {
    const stale = apiClient("/users/me").catch(error => error);
    await waitFor(() => expect(f.read).toHaveLength(1));
    f.hint("B");
    await waitFor(() => expect(f.read).toHaveLength(2));
    f.read[1]!("A");
    await waitFor(() => expect(auth.getState().isIdentityVerifying).toBe(false));
    f.read[0]!(Response.json({}, { status: 401 }));
    expect(isAccountAbort(await stale)).toBe(true);
    expect(auth.getState().user?.id).toBe("A");
    expect(f.posts).toEqual([]);
  } finally { f.dispose(); }
});
test("a logout refused before dispatch has no local effects or null hint", async () => {
  const f = fixture();
  try {
    f.hint("B");
    await waitFor(() => expect(f.read).toHaveLength(1));
    const rejected = await auth.getState().logout().catch(error => error);
    expect(isAccountAbort(rejected)).toBe(true);
    expect(auth.getState().user?.id).toBe("A");
    expect(f.posts).toEqual([]);
    f.read[0]!("B");
    await waitFor(() => expect(auth.getState().user?.id).toBe("B"));
    expect(f.posts).toEqual(["B"]);
  } finally { f.dispose(); }
});

test("Settings recovers its empty profile query after unchanged same-account verification without another read", async () => {
  const f = fixture();
  const confirmedA = auth.getState().user;
  const scope = createAccountQueryScope({ defaultOptions: { queries: { retry: false } } });
  const root = createRootRoute();
  const route = createRoute({ getParentRoute: () => root, path: "/settings", component: SettingsPage });
  const router = createRouter({ routeTree: root.addChildren([route]), history: createMemoryHistory({ initialEntries: ["/settings"] }) });
  try {
    render(<QueryClientProvider client={scope.getClient()}><TooltipProvider><RouterProvider router={router} /></TooltipProvider></QueryClientProvider>);
    await waitFor(() => expect(f.read).toHaveLength(1));
    expect(screen.queryByRole("textbox", { name: "Name" })).not.toBeInTheDocument();
    act(() => f.hint("B"));
    await waitFor(() => expect(f.read).toHaveLength(2));
    await act(async () => { f.read[0]!("A"); await tick(); });
    expect(scope.getClient().getQueryData(["user", "me"])).toBeUndefined();
    expect(isAccountAbort(scope.getClient().getQueryState(["user", "me"])?.error)).toBe(true);
    await act(async () => { f.read[1]!("A"); await tick(); });
    expect(auth.getState().user).toBe(confirmedA);
    expect(auth.getState().isIdentityVerifying).toBe(false);
    expect(await screen.findByRole("textbox", { name: "Name" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Email" })).toHaveValue("A@example.test");
    expect(scope.getClient().getQueryData(["user", "me"])).toBe(confirmedA);
    await tick();
    expect(f.read).toHaveLength(2);
    expect(f.posts).toEqual([]);
  } finally { f.dispose(); scope.dispose(); }
});
