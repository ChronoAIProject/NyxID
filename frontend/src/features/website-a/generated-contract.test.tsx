import { cleanup, render, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { authHref, siteHref } from "./auth-links";
import EnglishHeader from "./generated/en/WebsiteHeader";
import ChineseHeader from "./generated/zh/WebsiteHeader";
import EnglishSolutions from "./generated/en/SolutionsPage";
import ChineseSolutions from "./generated/zh/SolutionsPage";

vi.mock("./website-context", () => ({
  useWebsite: () => ({
    language: "en",
    route: "/solutions",
    menuOpen: true,
    authHref,
    siteHref,
    filterClass: () => "pill",
    isFilterSelected: () => false,
    solutionVisible: () => true,
  }),
}));
vi.mock("./interactions", () => ({ SolutionFilm: () => null }));

afterEach(() => {
  cleanup();
  window.history.replaceState({}, "", "/");
});

describe.each([
  ["en", EnglishHeader, EnglishSolutions, "Log in", "Ask NyxBot"],
  ["zh", ChineseHeader, ChineseSolutions, "登录", "交给 NyxBot"],
] as const)(
  "generated %s website",
  (language, Header, Solutions, login, cta) => {
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
