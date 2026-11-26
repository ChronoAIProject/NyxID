/**
 * Slab transitions run on the UI thread with one duration and the slab's own
 * ease-out curve: the folded Duo's rail capsule fades (its glass and icons)
 * and the wide window's sidebar open/close. Reduce Motion swaps at once.
 */
export const SLAB_MOTION_DURATION = 250;
export const SLAB_MOTION_EASING = [0.2, 0.8, 0.2, 1] as const;

/** The persistent sidebar's share of the window: its width shown (1), none hidden (0). */
export function sidebarInset(openWidth: number, shown: number) {
  "worklet";
  return openWidth * shown;
}
