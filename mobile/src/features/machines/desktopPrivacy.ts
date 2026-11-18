export type DesktopPrivacy = {
  preventScreenCaptureAsync(key: string): Promise<void>;
  allowScreenCaptureAsync(key: string): Promise<void>;
  enableAppSwitcherProtectionAsync(intensity: number): Promise<void>;
  disableAppSwitcherProtectionAsync(): Promise<void>;
};

/** Serialize native protection so a late cleanup cannot uncover a newer viewer. */
export function createDesktopPrivacyQueue() {
  let tail = Promise.resolve();
  return (action: () => Promise<void>) => {
    const result = tail.then(action, action);
    tail = result.catch(() => {});
    return result;
  };
}
export const desktopPrivacyQueue = createDesktopPrivacyQueue();
