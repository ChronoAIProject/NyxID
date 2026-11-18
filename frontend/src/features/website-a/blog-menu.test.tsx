import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { siteHref } from "./auth-links";
import { MenuBlogLink } from "./optional-features";
let language: "en" | "zh" = "en";
const query = "?code=INV&return_to=%2Fkeys&tag=one&tag=two";
vi.mock("./website-context", () => ({
  useWebsite: () => ({
    language,
    route: "/blog/scoped-keys",
    siteHref: (path: string) =>
      siteHref((language === "zh" ? "/zh" : "") + path, query),
  }),
}));
afterEach(cleanup);
it.each(["en", "zh"] as const)(
  "restores a contextual Blog menu link without losing auth query (%s)",
  (locale) => {
    language = locale;
    render(<MenuBlogLink />);
    const link = screen.getByRole("link");
    expect(link).toHaveAttribute("aria-current", "location");
    expect(link).toHaveClass("on");
    const url = new URL(
      link.getAttribute("href")!,
      "https://nyx.chrono-ai.fun",
    );
    expect(url.pathname).toBe(locale === "zh" ? "/zh/blog" : "/blog");
    expect([...url.searchParams]).toEqual([...new URLSearchParams(query)]);
  },
);
