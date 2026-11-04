import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRootRoute, createRoute, createRouter, Link, Outlet, RouterProvider } from "@tanstack/react-router";
import { useThemeStore } from "@/stores/theme-store";
import { HomeGlyph, AutomationsGlyph, MachinesGlyph, PluginsGlyph, ApprovalsGlyph } from "./sidebar-nav-glyphs";

let reduced = false;
const motionListeners = new Set<() => void>();
const frames = new Map<number, FrameRequestCallback>();
let nextFrame = 0;
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
  reduced = false;
  useThemeStore.setState({ motion: "system" });
  vi.stubGlobal("matchMedia", () => ({
    get matches() { return reduced; },
    addEventListener: (_: string, listener: () => void) => motionListeners.add(listener),
    removeEventListener: (_: string, listener: () => void) => motionListeners.delete(listener),
  }));
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    const id = ++nextFrame;
    frames.set(id, callback);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
});
afterEach(() => {
  act(() => vi.advanceTimersByTime(5000));
  cleanup();
  frames.clear();
  motionListeners.clear();
  useThemeStore.setState({ motion: "system" });
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
function advance(ms: number) {
  act(() => {
    vi.advanceTimersByTime(ms);
    const pending = [...frames.values()];
    frames.clear();
    pending.forEach(callback => callback(performance.now()));
  });
}
const glyphs = [HomeGlyph, AutomationsGlyph, MachinesGlyph, PluginsGlyph, ApprovalsGlyph];

describe.each(glyphs.map(Glyph => ({ name: Glyph.name, Glyph })))("activation-only $name", ({ Glyph }) => {
  it("ignores hover/focus, plays one cycle, restarts on activation and finishes at rest", () => {
    const { container } = render(<a href="#" onClick={event => event.preventDefault()}><Glyph />Destination</a>);
    const row = screen.getByRole("link");
    const svg = container.querySelector("svg")!;
    const rest = svg.innerHTML;
    fireEvent.pointerOver(row);
    fireEvent.pointerEnter(row);
    fireEvent.focus(row);
    expect(svg).not.toHaveAttribute("data-playing");
    expect(svg.innerHTML).toBe(rest);
    fireEvent.click(row);
    expect(svg).toHaveAttribute("data-playing", "true");
    const first = svg.getAttribute("data-playback-id");
    advance(700);
    fireEvent.click(row);
    expect(svg.getAttribute("data-playback-id")).not.toBe(first);
    advance(4500);
    expect(svg).not.toHaveAttribute("data-playing");
    expect(svg.innerHTML).toBe(rest);
    expect(frames.size).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("accepts a native keyboard click and Space without duplicate activation", () => {
    render(<a href="#" onClick={event => event.preventDefault()}><Glyph />Destination</a>);
    const row = screen.getByRole("link");
    row.focus();
    // Enter's native click has detail=0; real key activation is also covered
    // in the browser without coupling this virtual clock to userEvent timers.
    fireEvent.click(row, { detail: 0 });
    const svg = row.querySelector("svg")!;
    expect(svg).toHaveAttribute("data-playing", "true");
    const enter = Number(svg.getAttribute("data-playback-id"));
    fireEvent.keyDown(row, { key: " " });
    expect(Number(svg.getAttribute("data-playback-id"))).toBe(enter + 1);
  });

  it("keeps both system and in-app reduced motion at rest", () => {
    render(<button><Glyph />Destination</button>);
    const row = screen.getByRole("button"), svg = row.querySelector("svg")!;
    const rest = svg.innerHTML;
    reduced = true;
    fireEvent.click(row);
    expect(svg).not.toHaveAttribute("data-playing");
    reduced = false;
    useThemeStore.setState({ motion: "reduce" });
    fireEvent.click(row);
    expect(svg).not.toHaveAttribute("data-playing");
    useThemeStore.setState({ motion: "system" });
    fireEvent.click(row);
    advance(250);
    expect(svg).toHaveAttribute("data-playing");
    act(() => { reduced = true; motionListeners.forEach(listener => listener()); });
    expect(svg).not.toHaveAttribute("data-playing");
    expect(svg.innerHTML).toBe(rest);
    reduced = false;
    fireEvent.click(row);
    act(() => useThemeStore.setState({ motion: "reduce" }));
    expect(svg).not.toHaveAttribute("data-playing");
  });
});

it("lets Home's puffs finish before the chimney sinks to its lucide rest frame", () => {
  const { container } = render(<button><HomeGlyph />Home</button>);
  fireEvent.click(screen.getByRole("button"));
  const svg = container.querySelector("svg")!;
  expect(svg.querySelectorAll(".nav-glyph-smoke-puff")).toHaveLength(3);
  fireEvent.animationEnd(svg.querySelector(".nav-glyph-smoke-puff")!);
  expect(svg).toHaveAttribute("data-playing");
  fireEvent.animationEnd(svg.querySelector(".nav-glyph-chimney")!);
  expect(svg).not.toHaveAttribute("data-playing");
  expect(svg.querySelectorAll(".nav-glyph-smoke-puff")).toHaveLength(0);
});

it("resolves a replaced live host without remounting the glyph", () => {
  const { container } = render(<a href="#"><MachinesGlyph />Machines</a>);
  const svg = container.querySelector("svg")!, oldRow = container.querySelector("a")!;
  const next = document.createElement("a");
  next.href = "#next";
  oldRow.replaceWith(next);
  next.append(svg);
  fireEvent.click(next);
  expect(svg).toHaveAttribute("data-playing", "true");
  oldRow.append(svg);
  next.replaceWith(oldRow);
});

for (const rail of [false, true]) it(`keeps the activation clock after SPA navigation remounts the sidebar (rail=${rail})`, async () => {
  const root = createRootRoute({ component: Outlet });
  const SidebarPage = () => <><nav aria-label="Workspace"><Link to="/assistant/plugins" aria-label="Plugins"><PluginsGlyph />{!rail && "Plugins"}</Link><Link to="/assistant/approvals" aria-label="Approvals"><ApprovalsGlyph />{!rail && "Approvals"}</Link></nav><p>Workspace page</p></>;
  const routes = ["assistant/plugins", "assistant/approvals"].map(path => createRoute({ getParentRoute: () => root, path, component: () => <SidebarPage key={path} /> }));
  const router = createRouter({ routeTree: root.addChildren(routes), history: createMemoryHistory({ initialEntries: ["/assistant/plugins"] }) });
  await act(async () => { await router.load(); });
  render(<RouterProvider router={router} />);
  const before = screen.getByRole("link", { name: "Approvals" }).querySelector("svg")!;
  await act(async () => { fireEvent.click(screen.getByRole("link", { name: "Approvals" })); await router.load(); });
  expect(router.state.location.pathname).toBe("/assistant/approvals");
  const after = screen.getByRole("link", { name: "Approvals" }).querySelector("svg")!;
  expect(after).not.toBe(before);
  expect(after).toHaveAttribute("data-playing", "true");
  advance(750);
  expect(after.querySelectorAll("path")[1]!.getAttribute("d")).not.toBe("M9.00 12.00L11.00 14.00M11.00 14.00L15.00 10.00");
  advance(4250);
  expect(after).not.toHaveAttribute("data-playing");
  expect(after.querySelectorAll("path")[1]!.getAttribute("d")).toBe("M9.00 12.00L11.00 14.00M11.00 14.00L15.00 10.00");
});
