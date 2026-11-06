import { createContext } from "react";
export const GroupedDetailContext = createContext<{
  readonly container: HTMLElement | null;
  readonly visible: boolean;
  readonly level?: number;
}>({ container: null, visible: true });

const covers = new WeakMap<
  HTMLElement,
  { count: number; inert: boolean; hidden: string | null }
>();

/** Nested retained pages can cover the same controls and release them in either order. */
export function coverGroupedContent(element: HTMLElement) {
  const state = covers.get(element) ?? {
    count: 0,
    inert: element.inert,
    hidden: element.getAttribute("aria-hidden"),
  };
  state.count++;
  covers.set(element, state);
  element.inert = true;
  element.setAttribute("aria-hidden", "true");
  return () => {
    if (--state.count) return;
    element.inert = state.inert;
    if (state.hidden === null) element.removeAttribute("aria-hidden");
    else element.setAttribute("aria-hidden", state.hidden);
    covers.delete(element);
  };
}
