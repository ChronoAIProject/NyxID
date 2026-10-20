import { act, forwardRef } from "react";
import { renderToString } from "react-dom/server";
import { hydrateRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { WebsitePage } from "../website-a/website-page";
const location = vi.hoisted(() => ({ pathname: "/", searchStr: "", hash: "" }));
vi.mock("@tanstack/react-router", () => ({
  useLocation: () => location,
  Link: forwardRef<HTMLAnchorElement, Record<string, unknown>>(function Link(
    { to, search, hash, preload: _preload, ...props },
    ref,
  ) {
    void _preload;
    const query = new URLSearchParams(
      search as Record<string, string>,
    ).toString();
    return (
      <a
        {...props}
        ref={ref}
        href={`${to}${query ? `?${query}` : ""}${hash ? `#${hash}` : ""}`}
      />
    );
  }),
}));
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  window.history.replaceState({}, "", "/");
  document.body.replaceChildren();
  location.searchStr = "";
});
it.each([
  "?code=INVITE&return_to=%2Fkeys&utm_source=test",
  "?utm_source=reddit&return_to=%2Fkeys",
])(
  "hydrates canonical HTML before applying visitor context: %s",
  async (query) => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
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
    const tree = (
      <WebsitePage>
        <h1>Approved body</h1>
      </WebsitePage>
    );
    const container = document.createElement("div");
    container.innerHTML = renderToString(tree);
    document.body.append(container);
    expect(container.querySelector('a[href="/register"]')).toBeTruthy();
    window.history.replaceState({}, "", `/${query}`);
    location.searchStr = query;
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    let root: ReturnType<typeof hydrateRoot>;
    await act(async () => {
      root = hydrateRoot(container, tree);
    });
    const target = new URL(
      container.querySelector<HTMLAnchorElement>('a[href^="/register"]')!.href,
    );
    expect(target.searchParams.get("code")).toBe(
      query.includes("code=INVITE") ? "INVITE" : "NYX-QJGT6NHN",
    );
    expect(target.searchParams.get("return_to")).toBe("/keys");
    expect(
      errors.mock.calls.filter((args) =>
        /hydration|didn.t match|server rendered/i.test(args.join(" ")),
      ),
    ).toEqual([]);
    await act(async () => root!.unmount());
  },
);
