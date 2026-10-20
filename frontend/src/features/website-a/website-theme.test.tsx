import { forwardRef } from "react";
import { cleanup, render, screen } from "@testing-library/react";
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
      for (const pathname of ["/", "/", "/solutions", "/"]) {
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
});

vi.mock("./site-link", () => ({ SiteLink: forwardRef<HTMLAnchorElement, React.AnchorHTMLAttributes<HTMLAnchorElement>>(function Anchor(props,ref) { return <a {...props} ref={ref} />; }) }));
