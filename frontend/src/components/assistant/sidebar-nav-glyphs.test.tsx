import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Link,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import {
  HomeGlyph,
  PluginsGlyph,
  ApprovalsGlyph,
  MachinesGlyph,
} from "./sidebar-nav-glyphs";

let reduced = false;
let motionChanged: (() => void) | undefined;
beforeEach(() => {
  reduced = false;
  vi.stubGlobal("matchMedia", () => ({
    get matches() {
      return reduced;
    },
    addEventListener: (_: string, listener: () => void) => {
      motionChanged = listener;
    },
    removeEventListener: vi.fn(),
  }));
});
afterEach(() => {
  vi.unstubAllGlobals();
});

function glyph(label: string) {
  return screen.getByRole("link", { name: label }).querySelector("svg")!;
}

describe.each([false, true])(
  "workspace glyphs after navigation (rail=%s)",
  (rail) => {
    it("starts on hover without a click after an SPA navigation", async () => {
      const root = createRootRoute({
        component: () => (
          <>
            <nav aria-label={rail ? "Rail" : "Workspace"}>
              <Link to="/assistant/automations">Automations</Link>
              <Link to="/assistant/plugins" aria-label="Plugins">
                <PluginsGlyph />
                {!rail && "Plugins"}
              </Link>
              <Link to="/assistant/approvals" aria-label="Approvals">
                <ApprovalsGlyph />
                {!rail && "Approvals"}
              </Link>
            </nav>
            <Outlet />
          </>
        ),
      });
      const routes = ["automations", "plugins", "approvals"].map((path) =>
        createRoute({
          getParentRoute: () => root,
          path: `assistant/${path}`,
          component: () => <p>Workspace page</p>,
        }),
      );
      const router = createRouter({
        routeTree: root.addChildren(routes),
        history: createMemoryHistory({ initialEntries: ["/assistant/plugins"] }),
      });
      const user = userEvent.setup();
      render(<RouterProvider router={router} />);
      await user.click(
        await screen.findByRole("link", { name: "Automations" }),
      );
      await waitFor(() =>
        expect(router.state.location.pathname).toBe("/assistant/automations"),
      );
      for (const name of ["Plugins", "Approvals"]) {
        expect(glyph(name)).not.toHaveAttribute("data-playing");
        await user.hover(screen.getByRole("link", { name }));
        expect(glyph(name)).toHaveAttribute("data-playing", "true");
        expect(router.state.location.pathname).toBe("/assistant/automations");
        await user.unhover(screen.getByRole("link", { name }));
      }
    });
  },
);

it("resolves a replaced row without remounting its glyph and ignores child-to-child moves", () => {
  const { container } = render(
    <a href="#">
      <MachinesGlyph />
      <span>Machines</span>
    </a>,
  );
  const svg = container.querySelector("svg")!;
  const oldRow = container.querySelector("a")!;
  const next = document.createElement("a");
  next.href = "#next";
  oldRow.replaceWith(next);
  next.append(svg, document.createElement("span"));
  fireEvent.pointerOver(next, { relatedTarget: document.body });
  expect(svg).toHaveAttribute("data-playing", "true");
  fireEvent.animationIteration(svg.querySelector(".nav-glyph-heartbeat")!);
  fireEvent.pointerOver(next.lastChild!, { relatedTarget: svg });
  fireEvent.animationIteration(svg.querySelector(".nav-glyph-heartbeat")!);
  expect(svg).not.toHaveAttribute("data-playing");
  oldRow.prepend(svg);
  next.replaceWith(oldRow);
});

it("does not animate under reduced motion and stops an active frame loop when it changes", async () => {
  const user = userEvent.setup();
  render(
    <a href="#" aria-label="Plugins">
      <PluginsGlyph />
    </a>,
  );
  reduced = true;
  await user.hover(screen.getByRole("link"));
  expect(glyph("Plugins")).not.toHaveAttribute("data-playing");
  await user.unhover(screen.getByRole("link"));
  reduced = false;
  await user.hover(screen.getByRole("link"));
  expect(glyph("Plugins")).toHaveAttribute("data-playing");
  act(() => {
    reduced = true;
    motionChanged?.();
  });
  expect(glyph("Plugins")).not.toHaveAttribute("data-playing");
  expect(glyph("Plugins").querySelectorAll("rect")).toHaveLength(4);
});

it("clears hover when a retained glyph loses its old host before pointer-out", () => {
  const { container } = render(<a href="#"><HomeGlyph /></a>);
  const svg = container.querySelector("svg")!;
  const oldRow = container.querySelector("a")!;
  fireEvent.pointerOver(oldRow, { relatedTarget: document.body });
  expect(svg).toHaveAttribute("data-hovering", "true");
  const next = document.createElement("a");
  oldRow.replaceWith(next);
  next.append(svg);
  fireEvent.pointerOver(document.body);
  expect(svg).not.toHaveAttribute("data-hovering");
  oldRow.append(svg);
  next.replaceWith(oldRow);
});
