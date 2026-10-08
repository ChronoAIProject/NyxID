import { afterEach, expect, it, vi } from "vitest";
import { BUILD_UPDATE_VIEW_SWITCH, navigateWithBuildUpdate } from "./build-update-navigation";

afterEach(() => vi.restoreAllMocks());

it("signals only after successful navigation to a different view", async () => {
  const listener = vi.fn();
  window.addEventListener(BUILD_UPDATE_VIEW_SWITCH, listener);
  window.history.replaceState({}, "", "/assistant?c=old");
  let finish!: () => void;
  const navigation = navigateWithBuildUpdate(() => new Promise<void>((resolve) => { finish = resolve; }));
  expect(listener).not.toHaveBeenCalled();
  window.history.replaceState({}, "", "/assistant?c=new");
  finish();
  await navigation;
  expect(listener).toHaveBeenCalledOnce();
  await navigateWithBuildUpdate(async () => {});
  expect(listener).toHaveBeenCalledOnce();
  await expect(navigateWithBuildUpdate(async () => { throw new Error("blocked"); })).rejects.toThrow("blocked");
  expect(listener).toHaveBeenCalledOnce();
  window.removeEventListener(BUILD_UPDATE_VIEW_SWITCH, listener);
});
