import { beforeEach, expect, it, vi } from "vitest";
import { useBuildUpdateStore } from "./build-update-store";

const build = { buildId: "next", commit: null, assets: ["assets/next.js"] };
beforeEach(() => {
  localStorage.clear();
  useBuildUpdateStore.setState({ pending: null });
});

it("restores a pending update and its deadline from persistent storage", async () => {
  useBuildUpdateStore.getState().record(build);
  const pending = useBuildUpdateStore.getState().pending;
  const saved = localStorage.getItem("nyxid.build-update");
  useBuildUpdateStore.setState({ pending: null });
  localStorage.setItem("nyxid.build-update", saved!);
  await useBuildUpdateStore.persist.rehydrate();
  expect(useBuildUpdateStore.getState().pending).toEqual(pending);
});

it("retains the first deadline when a newer deployment supersedes the pending one", () => {
  const clock = vi.spyOn(Date, "now").mockReturnValue(1000);
  useBuildUpdateStore.getState().record(build);
  clock.mockReturnValue(2000);
  useBuildUpdateStore.getState().record({ ...build, buildId: "newer" });
  expect(useBuildUpdateStore.getState().pending).toEqual({ build: { ...build, buildId: "newer" }, detectedAt: 1000 });
  clock.mockRestore();
});

it("discards malformed persistent metadata", async () => {
  localStorage.setItem("nyxid.build-update", JSON.stringify({ state: { pending: { build: { buildId: "next" }, detectedAt: "invalid" } }, version: 0 }));
  await useBuildUpdateStore.persist.rehydrate();
  expect(useBuildUpdateStore.getState().pending).toBeNull();
});
