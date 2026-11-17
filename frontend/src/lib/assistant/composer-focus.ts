/**
 * Whether the composer may take focus without being asked to.
 *
 * On a touch device, focusing a textarea raises the on-screen keyboard over
 * half the screen. Opening a conversation to read it is the common case there,
 * so autofocus would cost a dismiss on nearly every visit; a pointer is the
 * closest available proxy for "typing is cheap here".
 */
export function autoFocusAllowed(): boolean {
  if (typeof window === "undefined") return false;
  // No matchMedia (jsdom, older engines) means no evidence of a touch device.
  if (typeof window.matchMedia !== "function") return true;
  return !window.matchMedia("(pointer: coarse)").matches;
}
