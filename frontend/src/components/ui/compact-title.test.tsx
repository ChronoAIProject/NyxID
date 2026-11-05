import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { CompactChrome, CompactScroll, CompactTitleProvider, StickyTitle } from "./compact-title";
import { useCompactTitle } from "@/lib/compact-title";
import { useVisualViewportBounds } from "@/lib/form-viewport";

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function observers() {
  const observed: { callback: IntersectionObserverCallback; element: Element }[] = [];
  vi.stubGlobal("IntersectionObserver", class {
    private callback: IntersectionObserverCallback;
    constructor(callback: IntersectionObserverCallback) { this.callback = callback; }
    observe(element: Element) { observed.push({ callback: this.callback, element }); }
    disconnect() {}
  });
  return (visible: boolean, kind?: string) => act(() => {
    const target = kind ? [...observed].reverse().find((entry) => entry.element.hasAttribute(kind))! : observed.at(-1)!;
    target.callback([{ target: target.element, isIntersecting: visible } as IntersectionObserverEntry], {} as IntersectionObserver);
  });
}
function Header() { const compact = useCompactTitle(); return <CompactChrome><p>{compact ? "Compact" : "Expanded"}</p><input aria-label="Search details" /></CompactChrome>; }
it("collapses from a sentinel and retains the same focused search input and scroll content", () => {
  const intersect = observers();
  render(<CompactTitleProvider><Header /><CompactScroll><p>Retained draft</p></CompactScroll></CompactTitleProvider>);
  const field = screen.getByRole("textbox"); field.focus();
  intersect(false, "data-collapse-sentinel");
  expect(screen.getByText("Compact")).toBeVisible();
  expect(field).toHaveFocus();
  expect(screen.getByRole("textbox")).toBe(field);
  expect(screen.getByText("Retained draft")).toBeInTheDocument();
  intersect(true, "data-collapse-sentinel");
  expect(screen.getByText("Compact")).toBeVisible();
  intersect(true, "data-expand-sentinel");
  expect(screen.getByText("Expanded")).toBeVisible();
  expect(field).toHaveFocus();
});
it("preserves one accessible heading and makes only the desktop mirror action tabbable", () => {
  const intersect = observers();
  vi.spyOn(window, "matchMedia").mockReturnValue({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() } as unknown as MediaQueryList);
  render(<StickyTitle title="Automations" actions={<button>Create automation</button>}><h1>Automations</h1><div data-title-actions=""><button>Create automation</button></div></StickyTitle>);
  intersect(false);
  expect(screen.getAllByRole("heading", { name: "Automations" })).toHaveLength(1);
  expect(document.querySelector<HTMLElement>("[data-title-actions]")!.inert).toBe(true);
  expect(document.querySelector("[data-sticky-title]")).toHaveAttribute("data-visible", "true");
  intersect(true);
  expect(document.querySelector<HTMLElement>("[data-title-actions]")!.inert).toBe(false);
});
it("constrains a mobile sheet to the visual viewport above the keyboard", () => {
  const viewport = new EventTarget() as VisualViewport;
  Object.defineProperties(viewport, { height: { value: 480, configurable: true }, offsetTop: { value: 30, configurable: true } });
  vi.stubGlobal("visualViewport", viewport);
  vi.spyOn(window, "matchMedia").mockReturnValue({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() } as unknown as MediaQueryList);
  function Sheet() { const style = useVisualViewportBounds(); return <div data-testid="sheet" style={style}><div>Scroll area</div><footer>Save changes · Discard</footer></div>; }
  render(<Sheet />);
  expect(screen.getByTestId("sheet")).toHaveStyle({ top: "30px", maxHeight: "480px", height: "auto" });
  Object.defineProperty(viewport, "height", { value: 400 });
  act(() => viewport.dispatchEvent(new Event("resize")));
  expect(screen.getByTestId("sheet")).toHaveStyle({ maxHeight: "400px" });
  expect(screen.getByText("Save changes · Discard")).toBeVisible();
});
