import { forwardRef } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useApplyTheme, useResolvedTheme } from "@/hooks/use-theme";
import { useThemeStore } from "@/stores/theme-store";
import { WebsitePage } from "./website-page";

const location = vi.hoisted(() => ({ pathname: "/" }));
vi.mock("@tanstack/react-router", () => ({ useLocation: () => location }));
vi.mock("./generated/en", () => ({ default: () => <h1>NyxBot</h1> }));
vi.mock("./generated/zh", () => ({ default: () => <h1>NyxBot</h1> }));

function ApplicationTheme() {
  useApplyTheme();
  return <output data-testid="app-theme">{useResolvedTheme()}</output>;
}

afterEach(() => {
  cleanup();
  location.pathname = "/";
  localStorage.removeItem("nyxid-theme");
  useThemeStore.setState({ mode: "system", systemPrefersDark: false });
});

describe("independent public website theme", () => {
  it.each(["system", "light", "dark"] as const)(
    "keeps the public choice across opens and route returns with app mode %s",
    async (mode) => {
      useThemeStore.setState({ mode, systemPrefersDark: false });
      const saved = localStorage.getItem("nyxid.theme");
      const expectedApp = mode === "dark" ? "dark" : "light";
      localStorage.setItem("nyxid-theme", "light");
      // A public theme choice follows the reader without changing their app preference.
      for (const [index, pathname] of ["/nyxbot", "/nyxbot", "/solutions", "/nyxbot"].entries()) {
        location.pathname = pathname;
        const website = render(<WebsitePage />);
        await screen.findByRole("heading", { name: "NyxBot" });
        expect(website.container.querySelector(".website-a")).toHaveAttribute(
          "data-theme",
          index === 0 ? "light" : "dark",
        );
        if (index === 0) {
          fireEvent.click(screen.getByRole("button", { name: "Switch to night mode" }));
          expect(website.container.querySelector(".website-a")).toHaveAttribute("data-theme", "dark");
          expect(localStorage.getItem("nyxid-theme")).toBe("dark");
        }
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
