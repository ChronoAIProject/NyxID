import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RouterProvider } from "@tanstack/react-router";
import { publicRouter } from "./router";

// Navigation runs in happy-dom, which does not paint or implement hit testing.
// The real motion lifecycle is checked in the browser, independently of routing.
vi.mock("../website-a/nyx-home/nyx-home-motion", () => ({
  startNyxHome: () => () => {},
}));

beforeEach(() => {
  vi.stubGlobal("matchMedia", () => ({
    matches: true,
    addEventListener() {},
    removeEventListener() {},
  }));
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  window.history.replaceState({}, "", "/");
});
const query =
  "?utm_campaign=2026&flag=true&empty=null&json=%7B%22a%22%3A1%7D&encoded=a%2Bb%20c&tag=first&x=middle&tag=second&code=INVITE&return_to=%2Fkeys%3Ftab%3Dactive";
it("F02 keeps ordered string query values with the real public Router", async () => {
  window.history.replaceState({}, "", "/" + query);
  const router = publicRouter();
  await router.load();
  const view = render(<RouterProvider router={router} />);
  const entries = Array.from(new URLSearchParams(query));
  await screen.findByRole("heading", { level: 1 }, { timeout: 5000 });
  for (const path of ["/security", "/solutions", "/zh/solutions"]) {
    const selector = path.startsWith("/zh")
      ? '.public-footer a[lang="zh-CN"]'
      : `.public-footer a[href^="${path}?"]`;
    const link = view.container.querySelector<HTMLAnchorElement>(selector)!;
    expect(Array.from(new URL(link.href).searchParams)).toEqual(entries);
    await act(async () => {
      fireEvent.click(link);
    });
    if (path.startsWith("/zh")) {
      // Switching locale uses a real document URL. JSDOM does not follow ordinary anchors.
      await act(async () => router.navigate({ to: new URL(link.href).pathname as never, search: router.state.location.search as never }));
    }
    await waitFor(() => expect(router.state.location.pathname).toBe(path));
    if (path === "/security")
      await screen.findByRole("heading", { level: 1, name: /Keep credentials out of the agent/ });
    expect(
      Array.from(new URLSearchParams(router.state.location.searchStr)),
    ).toEqual(entries);
  }
});
it("F03 retries the failed Docs loader when its source recovers", async () => {
  let calls = 0;
  const doc =
    "---\ntitle: Recovered documentation\ndescription: A recovered document\n---\n\n## Ready\n\nApproved body is available again.";
  const router = publicRouter("/docs/ai/guides/agent-isolation", async () => {
    if (++calls <= 2) throw new Error("Documentation unavailable (503)");
    return doc;
  });
  await router.load();
  render(<RouterProvider router={router} />);
  await screen.findByRole("button", { name: "Retry" });
  expect(calls).toBe(2);
  fireEvent.click(await screen.findByRole("button", { name: "Retry" }));
  await screen.findByRole(
    "heading",
    { name: "Recovered documentation" },
    { timeout: 2000 },
  );
  expect(calls).toBe(3);
});
it("F04 closes Show more after navigation and history", async () => {
  const router = publicRouter();
  await router.load();
  const view = render(<RouterProvider router={router} />);
  await screen.findByRole("heading", { level: 1 }, { timeout: 5000 });
  const menu = () => view.container.querySelector<HTMLDetailsElement>(".public-more")!;
  fireEvent.click(menu().querySelector("summary")!);
  expect(menu().open).toBe(true);
  await act(async () => {
    fireEvent.click(
      within(menu()).getByRole("link", {
        name: /^Security /,
      }),
    );
  });
  await waitFor(() => expect(router.state.location.pathname).toBe("/security"));
  await waitFor(() =>
    expect(menu().open).toBe(false),
  );
  fireEvent.click(menu().querySelector("summary")!);
  expect(menu().open).toBe(true);
  await act(async () => router.history.back());
  await waitFor(() => expect(router.state.location.pathname).toBe("/"));
  await waitFor(() => expect(menu().open).toBe(false));
});
