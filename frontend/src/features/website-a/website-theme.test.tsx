import { forwardRef } from "react";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useApplyTheme, useResolvedTheme } from "@/hooks/use-theme";
import { useThemeStore } from "@/stores/theme-store";
import { WebsitePage } from "./website-page";

const location = vi.hoisted(() => ({ pathname: "/" }));
vi.mock("@tanstack/react-router", () => ({ useLocation: () => location }));
vi.mock("./generated/en", async () => {
  const { HeroTitle } = await import("./visuals/hero");
  return { default: HeroTitle };
});
vi.mock("./generated/zh", async () => {
  const { HeroTitle } = await import("./visuals/hero");
  return { default: HeroTitle };
});
vi.mock("./visuals/motion", async (original) => ({
  ...(await original<typeof import("./visuals/motion")>()),
  useMotionTarget: () => ({ active: true, allowed: true, seen: true }),
}));
vi.mock("./visuals/react-bits/GradientText", () => ({
  default: ({
    colors,
    children,
  }: {
    colors: string[];
    children: React.ReactNode;
  }) => (
    <span data-testid="gradient" data-colors={colors.join(",")}>
      {children}
    </span>
  ),
}));

function ApplicationTheme() {
  useApplyTheme();
  return <output data-testid="app-theme">{useResolvedTheme()}</output>;
}

afterEach(() => {
  cleanup();
  location.pathname = "/";
  useThemeStore.setState({ mode: "system", systemPrefersDark: false });
});

describe("independent public website theme", () => {
  it.each(["system", "light", "dark"] as const)(
    "stays dark across opens, refresh and route return with app mode %s",
    async (mode) => {
      useThemeStore.setState({ mode, systemPrefersDark: false });
      const saved = localStorage.getItem("nyxid.theme");
      const expectedApp = mode === "dark" ? "dark" : "light";
      // The English home page has its own day/night switch (tested below); the other pages stay dark.
      for (const pathname of ["/nyxbot", "/nyxbot", "/solutions", "/nyxbot"]) {
        location.pathname = pathname;
        const website = render(<WebsitePage />);
        await screen.findByTestId("gradient");
        expect(website.container.querySelector(".website-a")).toHaveAttribute(
          "data-theme",
          "dark",
        );
        expect(screen.getByTestId("gradient")).toHaveAttribute(
          "data-colors",
          "#A672FB,#5E00F5",
        );
        website.unmount();
        expect(localStorage.getItem("nyxid.theme")).toBe(saved);
        expect(useThemeStore.getState().mode).toBe(mode);

        const app = render(<ApplicationTheme />);
        expect(screen.getByTestId("app-theme")).toHaveTextContent(expectedApp);
        expect(document.documentElement).toHaveClass(`theme-${expectedApp}`);
        app.unmount();
        expect(localStorage.getItem("nyxid.theme")).toBe(saved);
      }
    },
  );

  it.each(["light", "dark"] as const)(
    "home page keeps its own %s mode on the page and never changes the app's",
    async (homeMode) => {
      useThemeStore.setState({ mode: "system", systemPrefersDark: false });
      localStorage.setItem("nyxid-theme", homeMode);
      const saved = localStorage.getItem("nyxid.theme");
      location.pathname = "/";
      const website = render(<WebsitePage />);
      const home = await waitFor(() => {
        const element = website.container.querySelector(".nyx-home");
        expect(element).not.toBeNull();
        return element as HTMLElement;
      });
      expect(home).toHaveAttribute("data-theme", homeMode);
      expect(website.container.querySelector(".website-a")).toBeNull();
      expect(document.documentElement).not.toHaveAttribute("data-theme");
      website.unmount();
      localStorage.removeItem("nyxid-theme");
      expect(localStorage.getItem("nyxid.theme")).toBe(saved);
      expect(useThemeStore.getState().mode).toBe("system");
    },
  );
});

// The home page's animations need a real browser (WebGL, layout); they are not part of this test.
vi.mock("./nyx-home/nyx-home-motion", () => ({
  startNyxHome: () => () => {},
}));

vi.mock("./site-link", () => ({ SiteLink: forwardRef<HTMLAnchorElement, React.AnchorHTMLAttributes<HTMLAnchorElement>>(function Anchor(props,ref) { return <a {...props} ref={ref} />; }) }));
