import { forwardRef } from "react";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import {
  afterAll,
  afterEach,
  beforeAll,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { allDocPages } from "@/features/docs/manifest";
import { isPublicPath } from "@/lib/public-paths";
import routes from "./generated/routes.json";
import { WebsitePage } from "./website-page";
import { parsePublicPath } from "../public-site/publication";

const location = vi.hoisted(() => ({ pathname: "/", searchStr: "" }));
vi.mock("@tanstack/react-router", () => ({
  useLocation: () => location,
  Link: forwardRef<HTMLAnchorElement, Record<string, unknown>>(
    function TestLink({ to, search, hash, preload: _preload, ...props }, ref) {
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
    },
  ),
}));

const publicDestinations = new Set([
  ...routes,
  "/login",
  "/register",
  "/privacy",
  "/terms",
  "/docs",
  ...allDocPages().map(({ slug }) => `/docs/${slug}`),
]);
const queryFor = (language: string) =>
  `?lang=${language}&code=NYX-TEST1234&return_to=%2Fkeys%3Ftab%3Dactive&utm_source=page-audit`;

beforeAll(() => {
  // Run real page components with motion suppressed; no browser, GPU or network.
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({
      matches: true,
      media: "(prefers-reduced-motion: reduce)",
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  );
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
});

afterEach(() => {
  cleanup();
  window.history.replaceState({}, "", "/");
});
afterAll(() => vi.unstubAllGlobals());

async function openPage(pathname: string, language = "en") {
  location.pathname =
    language === "zh" ? `/zh${pathname === "/" ? "" : pathname}` : pathname;
  location.searchStr = queryFor(language);
  window.history.replaceState({}, "", location.pathname + location.searchStr);
  const result = render(<WebsitePage />);
  await screen.findByRole("heading", { level: 1 }, { timeout: 5000 });
  return result;
}

describe.each(["en", "zh"])("public page matrix: %s", (language) => {
  it.each(routes)(
    "renders %s with real destinations and preserved auth context",
    async (pathname) => {
      expect(isPublicPath(pathname)).toBe(true);
      const { container } = await openPage(pathname, language);
      expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
      expect(
        container.querySelector("main")?.textContent?.length,
      ).toBeGreaterThan(pathname === "/blog" ? 50 : 100);
      expect(container.querySelector("nav")).toBeTruthy();
      expect(container.querySelector("footer")).toBeTruthy();
      for (const anchor of container.querySelectorAll<HTMLAnchorElement>("a")) {
        const href = anchor.getAttribute("href");
        expect(href, `${pathname}: ${anchor.textContent}`).toBeTruthy();
        expect(href).not.toMatch(/^(?:javascript:|#\/)/i);
        const destination = new URL(href!, window.location.href);
        if (destination.origin !== window.location.origin) {
          expect(["https:", "mailto:"]).toContain(destination.protocol);
          continue;
        }
        expect(
          publicDestinations.has(parsePublicPath(destination.pathname).route),
          `${pathname} -> ${href}`,
        ).toBe(true);
        if (href!.startsWith("#")) {
          expect(
            container.querySelector(`[id="${destination.hash.slice(1)}"]`),
            `${pathname} -> ${href}`,
          ).toBeTruthy();
        } else {
          expect(destination.searchParams.get("code"), href!).toBe(
            "NYX-TEST1234",
          );
          expect(destination.searchParams.get("return_to"), href!).toBe(
            "/keys?tab=active",
          );
          if (["/login", "/register"].includes(destination.pathname))
            expect(destination.searchParams.get("lang"), href!).toBe(language);
        }
      }
      expect(
        container.querySelectorAll('a[href^="/login?"]').length,
      ).toBeGreaterThan(0);
      expect(
        container.querySelectorAll('a[href^="/register?"]').length,
      ).toBeGreaterThan(0);
    },
  );

  it.each([
    ["/nyxbot", "/nyxbot"],
    ["/solutions/restaurant-manager", "/solutions"],
    ["/blog/what-is-nyxid", "/blog"],
  ])("marks the navbar section for %s", async (pathname, section) => {
    const { container } = await openPage(pathname, language);
    const links = [
      ...container.querySelectorAll<HTMLAnchorElement>("nav a.nl"),
    ];
    const active = links.filter((link) => link.getAttribute("aria-current"));
    expect(active).toHaveLength(1);
    expect(parsePublicPath(active[0]?.pathname ?? "/").route).toBe(section);
    expect(active[0]).toHaveClass("on");
    expect(links.filter((link) => link.classList.contains("on"))).toHaveLength(
      1,
    );
  });
});

it("filters solutions by category and channel without losing custom-solution navigation", async () => {
  const { container } = await openPage("/solutions");
  const cards = () =>
    [
      ...container.querySelectorAll<HTMLElement>("#solGrid > [data-category]"),
    ].filter((el) => !el.hidden);
  expect(cards()).toHaveLength(9);
  const category = container.querySelector<HTMLButtonElement>(
    '#catF button[data-v]:not([data-v="all"])',
  )!;
  fireEvent.click(category);
  expect(cards().length).toBeGreaterThan(0);
  expect(
    cards().every((card) => card.dataset.category === category.dataset.v),
  ).toBe(true);
  const channel = within(container.querySelector("#chF")!).getByRole("button", {
    name: "Telegram",
  });
  fireEvent.click(channel);
  expect(cards().length).toBeGreaterThan(0);
  expect(
    cards().every((card) =>
      card.dataset.channels?.split(",").includes("Telegram"),
    ),
  ).toBe(true);
  expect(
    container.querySelector('#solGrid a[href^="/nyxbot?"]'),
  ).not.toHaveAttribute("hidden");
});

it("filters integrations, reports no results, and restores the catalogue", async () => {
  const { container } = await openPage("/integrations");
  const search = container.querySelector<HTMLInputElement>("#intQ")!;
  const cards = () =>
    [...container.querySelectorAll<HTMLElement>(".int[data-name]")].filter(
      (el) => !el.hidden,
    );
  const total = cards().length;
  expect(total).toBeGreaterThan(0);
  fireEvent.change(search, { target: { value: "github" } });
  expect(cards().length).toBeGreaterThan(0);
  expect(cards().every((card) => card.dataset.name?.includes("github"))).toBe(
    true,
  );
  fireEvent.change(search, { target: { value: "no-such-integration-qa" } });
  expect(cards()).toHaveLength(0);
  expect(screen.getByRole("status")).toHaveTextContent("Showing 0 of");
  fireEvent.change(search, { target: { value: "" } });
  expect(cards()).toHaveLength(total);
});

it("withholds unapproved articles and newsletter promises", async () => {
  const { container } = await openPage("/blog");
  expect(container.querySelector(".featpost")).toBeNull();
  expect(container.querySelector("#blogF")).toBeNull();
  expect(container.querySelector("#blogGrid")).toHaveTextContent(/review/);
  expect(container.querySelector(".nl-form")).toBeNull();
});

it("offers a real locale URL preserving business query and nested return", async () => {
  const { container } = await openPage("/contact");
  const link = within(container.querySelector("nav")!).getByRole("link", {
    name: "中文",
  });
  const url = new URL(link.getAttribute("href")!, window.location.href);
  expect(url.pathname).toBe("/zh/contact");
  expect(url.searchParams.has("lang")).toBe(false);
  expect(url.searchParams.get("return_to")).toBe("/keys?tab=active");
});
