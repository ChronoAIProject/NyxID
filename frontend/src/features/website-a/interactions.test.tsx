import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AudienceSwitcher, SolutionFilm } from "./interactions";

vi.mock("./website-context", () => ({
  useWebsite: () => ({ language: "en" }),
}));
let reduced = false;
let disconnect: ReturnType<typeof vi.fn>;
const items = ["Business", "AI enthusiasts", "Developers"].map((label, i) => ({
  id: String(i),
  label,
  content: <p>{label} content</p>,
}));
beforeEach(() => {
  vi.useFakeTimers();
  reduced = false;
  disconnect = vi.fn();
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      observe() {}
      disconnect = disconnect;
    },
  );
  vi.stubGlobal("matchMedia", () => ({
    matches: reduced,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }));
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  window.history.replaceState({}, "", "/");
});

describe("original audience showcase", () => {
  it("rotates every nine seconds and releases timers and visibility observer on unmount", () => {
    const { unmount } = render(<AudienceSwitcher items={items} />);
    expect(screen.getByRole("tab", { name: "Business" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    act(() => vi.advanceTimersByTime(9000));
    expect(screen.getByRole("tab", { name: "AI enthusiasts" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    unmount();
    expect(vi.getTimerCount()).toBe(0);
    expect(disconnect).toHaveBeenCalledOnce();
  });
  it("pauses while hovered, resumes, then stops after explicit selection", () => {
    const { container } = render(<AudienceSwitcher items={items} />);
    const area = container.firstElementChild!;
    fireEvent.mouseEnter(area);
    act(() => vi.advanceTimersByTime(12000));
    expect(screen.getByRole("tab", { name: "Business" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    fireEvent.mouseLeave(area);
    act(() => vi.advanceTimersByTime(9000));
    expect(screen.getByRole("tab", { name: "AI enthusiasts" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    fireEvent.click(screen.getByRole("tab", { name: "Developers" }));
    act(() => vi.advanceTimersByTime(20000));
    expect(screen.getByRole("tab", { name: "Developers" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });
  it("supports roving keyboard focus and stops automatic rotation on focus", () => {
    render(<AudienceSwitcher items={items} />);
    const first = screen.getByRole("tab", { name: "Business" });
    fireEvent.focus(first);
    act(() => vi.advanceTimersByTime(10000));
    expect(first).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(first, { key: "ArrowRight" });
    expect(screen.getByRole("tab", { name: "AI enthusiasts" })).toHaveFocus();
    expect(screen.getByRole("tabpanel")).toHaveTextContent(
      "AI enthusiasts content",
    );
  });
  it("shows a complete, non-rotating panel for reduced motion", () => {
    reduced = true;
    render(<AudienceSwitcher items={items} />);
    act(() => vi.advanceTimersByTime(20000));
    expect(screen.getByRole("tab", { name: "Business" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tabpanel")).toHaveClass("is-stopped");
  });
});

it("pauses and resumes the five-workflow film with its explicit control", () => {
  render(<SolutionFilm />);
  fireEvent.focus(screen.getByRole("button", { name: "Pause" }));
  fireEvent.click(screen.getByRole("button", { name: "Pause" }));
  expect(
    screen.getByRole("button", { name: "Play" }),
  ).toHaveAttribute("aria-pressed", "true");
  act(() => vi.advanceTimersByTime(13200));
  expect(
    screen.getByRole("button", { name: "Restaurant manager" }),
  ).toHaveAttribute("aria-pressed", "true");
  fireEvent.click(screen.getByRole("button", { name: "Play" }));
  act(() => vi.advanceTimersByTime(6600));
  expect(
    screen.getByRole("button", { name: "24-hour help desk" }),
  ).toHaveAttribute("aria-pressed", "true");
});
