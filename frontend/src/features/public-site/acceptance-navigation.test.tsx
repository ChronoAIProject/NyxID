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
      ? 'nav a[data-lang="zh"]'
      : `nav a[data-nav="${path.slice(1)}"]`;
    const link = view.container.querySelector<HTMLAnchorElement>(selector)!;
    expect(Array.from(new URL(link.href).searchParams)).toEqual(entries);
    await act(async () => {
      fireEvent.click(link);
    });
    await waitFor(() => expect(router.state.location.pathname).toBe(path));
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
it("F04 closes the menu after current-document navigation and history", async () => {
  const router = publicRouter();
  await router.load();
  const view = render(<RouterProvider router={router} />);
  await screen.findByRole("heading", { level: 1 }, { timeout: 5000 });
  const burger = () =>
    view.container.querySelector<HTMLButtonElement>("#burger")!;
  fireEvent.click(burger());
  expect(burger()).toHaveAttribute("aria-expanded", "true");
  await act(async () => {
    fireEvent.click(
      within(view.container.querySelector("nav")!).getByRole("link", {
        name: "Security",
      }),
    );
  });
  await waitFor(() => expect(router.state.location.pathname).toBe("/security"));
  expect(burger()).toHaveAttribute("aria-expanded", "false");
});
