import { createContext, useContext } from "react";
export interface ScrollState { active: boolean; level: number; compact: boolean }
export interface ChromeSize { expanded: number; current: number }
export const CompactTitleContext = createContext<{
  compact: boolean; gap: number;
  scroll: (id: string, state: ScrollState | null) => void;
  chrome: (id: string, size: ChromeSize | null) => void;
} | null>(null);


export function useCompactTitle() { return useContext(CompactTitleContext)?.compact ?? false; }
export function scrollOwner(element: HTMLElement): HTMLElement | null {
  for (let parent = element.parentElement; parent; parent = parent.parentElement)
    if (/(auto|scroll)/.test(getComputedStyle(parent).overflowY)) return parent;
  return null;
}
