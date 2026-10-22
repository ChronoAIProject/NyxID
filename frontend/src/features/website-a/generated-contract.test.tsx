import { forwardRef } from "react";
import { cleanup, render, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { authHref, siteHref } from "./auth-links";
import EnglishHeader from "./generated/en/WebsiteHeader";
import ChineseHeader from "./generated/zh/WebsiteHeader";
import EnglishSolutions from "./generated/en/SolutionsPage";
import ChineseSolutions from "./generated/zh/SolutionsPage";

const navState = vi.hoisted(() => ({ route: "/solutions" }));
vi.mock("./website-context", () => ({
  useWebsite: () => ({
    language: "en",
    route: navState.route,
    menuOpen: true,
    authHref,
    siteHref,
    localeHref: () => "/zh",
    filterClass: () => "pill",
    isFilterSelected: () => false,
    solutionVisible: () => true,
  }),
}));
vi.mock("./interactions", () => ({ SolutionFilm: () => null }));

afterEach(() => {
  cleanup();
  window.history.replaceState({}, "", "/");
  navState.route = "/solutions";
});

describe.each([
  ["en", EnglishHeader, EnglishSolutions, "Log in", "Ask NyxBot"],
  ["zh", ChineseHeader, ChineseSolutions, "登录", "交给 NyxBot"],
] as const)(
  "generated %s website",
  (language, Header, Solutions, login, cta) => {
    it.each([
      ["/nyxbot", "nyxbot", "page"],
      ["/solutions", "solutions", "page"],
      ["/how-it-works", "how-it-works", "page"],
      ["/security", "security", "page"],
      ["/agents", "agents", "page"],
      ["/pricing", "pricing", "page"],
      ["/blog", "blog", "page"],
      ["/solutions/restaurant-manager", "solutions", "location"],
      ["/blog/scoped-keys", "blog", "location"],
    ])(
      "marks the current column for %s without changing link context",
      (route, column, current) => {
        navState.route = route!;
        const query = `?lang=${language}&code=QA-INVITE&return_to=%2Fkeys`;
        window.history.replaceState({}, "", `${route}${query}`);
        const { container } = render(<Header />);
        const active = container.querySelectorAll("a.nl.on");
        if (column === "blog") {
          expect(container.querySelector('[data-nav="blog"]')).toBeNull();
          expect(active).toHaveLength(0);
          return;
        }
        expect(active).toHaveLength(1);
        expect(active[0]).toHaveAttribute("data-nav", column);
        expect(active[0]).toHaveAttribute("aria-current", current);
        expect(active[0]).toHaveAttribute("href", `/${column}${query}`);
        expect(
          container.querySelectorAll("a[data-nav][aria-current]"),
        ).toHaveLength(1);
      },
    );

    it("does not select lookalike prefixes or unrelated pages", () => {
      for (const route of [
        "/solutions-old",
        "/blogger",
        "/",
        "/contact",
        "/integrations",
      ]) {
        navState.route = route;
        const { container, unmount } = render(<Header />);
        expect(container.querySelectorAll("a.nl.on")).toHaveLength(0);
        expect(
          container.querySelectorAll("a[data-nav][aria-current]"),
        ).toHaveLength(0);
        unmount();
      }
    });

    it("offers a real login link inside the expanded menu with the full auth query", () => {
      const query = `?lang=${language}&code=QA-INVITE&return_to=%2Fkeys%3Ftab%3Dactive&utm_source=qa`;
      window.history.replaceState({}, "", `/${query}`);
      const { container } = render(<Header />);
      const menu = container.querySelector<HTMLElement>("#website-nav-links")!;
      const link = within(menu).getByRole("link", { name: login });
      expect(link).toHaveAttribute("href", `/login${query}`);
      expect(link).not.toHaveAttribute("tabindex", "-1");
      expect(link).not.toHaveClass("hide-m");
    });

    it("retains the custom solution CTA and its NyxBot destination", () => {
      window.history.replaceState(
        {},
        "",
        `/solutions?lang=${language}&code=QA-INVITE`,
      );
      const { container } = render(<Solutions />);
      const grid = container.querySelector<HTMLElement>("#solGrid")!;
      const card = within(grid).getByRole("link", { name: new RegExp(cta) });
      expect(card).toHaveAttribute(
        "href",
        `/nyxbot?lang=${language}&code=QA-INVITE`,
      );
      expect(card).toHaveStyle({
        borderStyle: "dashed",
        alignItems: "center",
        justifyContent: "center",
        textAlign: "center",
      });
    });
  },
);

vi.mock("./site-link", () => ({
  SiteLink: forwardRef<
    HTMLAnchorElement,
    React.AnchorHTMLAttributes<HTMLAnchorElement>
  >(function Anchor(props, ref) {
    return <a {...props} ref={ref} />;
  }),
}));
