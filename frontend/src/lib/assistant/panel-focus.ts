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
  const target = activeDialog?.querySelector<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex="0"]') ??
    document.querySelector<HTMLElement>('[data-assistant-account-fallback]');
  if (canRestoreFocus(target)) target.focus();
}
