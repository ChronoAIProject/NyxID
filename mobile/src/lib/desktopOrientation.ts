type ScreenOrientation = Pick<
  typeof import("expo-screen-orientation"),
  "lockAsync" | "OrientationLock"
>;

/** A late landscape lock must finish before the departing viewer restores portrait. */
export function createDesktopOrientation(
  load: () => Promise<ScreenOrientation> = () => import("expo-screen-orientation"),
) {
  let tail = Promise.resolve();
  function lock(target: "LANDSCAPE" | "PORTRAIT_UP"): Promise<void> {
    const result = tail.then(async () => {
      const native = await load();
      await native.lockAsync(native.OrientationLock[target]);
    });
    // Keep failures visible to this caller without poisoning later cleanup/retry.
    tail = result.catch(() => {});
    return result;
  }
  return {
    lockDesktopLandscape: () => lock("LANDSCAPE"),
    restorePortrait: () => lock("PORTRAIT_UP"),
  };
}

// Load the native module only when used: an older dev client can keep running
// management pages until the new binary has been installed.
export const { lockDesktopLandscape, restorePortrait } = createDesktopOrientation();
