export function canRestoreFocus(element: Element | null | undefined): element is HTMLElement {
  return element instanceof HTMLElement && element.isConnected &&
    !element.closest('[hidden], [inert], [aria-hidden="true"]') &&
    !element.matches(':disabled') && element.tabIndex >= 0 &&
    getComputedStyle(element).visibility !== "hidden" && visibleAncestors(element);
}

function visibleAncestors(element: HTMLElement) {
  for (let ancestor: HTMLElement | null = element; ancestor; ancestor = ancestor.parentElement) {
    if (getComputedStyle(ancestor).display === "none") return false;
  }
  return true;
}

export function restorePanelFocus(opener?: HTMLElement | null) {
  if (canRestoreFocus(opener)) {
    opener.focus();
    if (document.activeElement === opener) return;
  }
  const dialogs = Array.from(document.querySelectorAll<HTMLElement>('[role="dialog"][data-state="open"]'));
  const activeDialog = dialogs.at(-1);
  // Prefer the sidebar's account button; on mobile, where it lives in the
  // closed drawer, fall back to the drawer opener.
  const target = activeDialog?.querySelector<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex="0"]') ??
    ["menu", "drawer"].flatMap((kind) =>
      Array.from(document.querySelectorAll<HTMLElement>(`[data-assistant-account-fallback="${kind}"]`)),
    ).find(canRestoreFocus);
  if (canRestoreFocus(target)) target.focus();
}
