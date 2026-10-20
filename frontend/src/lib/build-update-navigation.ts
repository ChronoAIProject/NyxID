export const BUILD_UPDATE_VIEW_SWITCH = "nyxid:build-update-view-switch";

/** Only deliberate view changes participate; conversation adoption does not. */
export async function navigateWithBuildUpdate(navigate: () => Promise<void>) {
  const previous = window.location.href;
  await navigate();
  if (window.location.href !== previous) {
    window.dispatchEvent(new Event(BUILD_UPDATE_VIEW_SWITCH));
  }
}
