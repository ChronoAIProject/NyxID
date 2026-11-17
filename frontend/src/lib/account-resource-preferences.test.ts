import { beforeEach, expect, it } from "vitest";
import { bindAccountResourcePreferences } from "./account-resource-preferences";
import { useMachinePaneStore } from "@/stores/machine-pane-store";
import { transitionAssistantIdentity } from "./assistant/identity";

const key = "nyxid:last-picked-agent-key";
const collapse = "nyxid:desktop-collapsed:conversation:node:context";
beforeEach(() => { localStorage.clear(); transitionAssistantIdentity("A"); useMachinePaneStore.setState({ ownerUserId: null, hidden: [] }); });
it.each(["A", "B", null])("restores resource preferences only for their persisted owner (%s)", async (owner) => {
  bindAccountResourcePreferences("A");
  localStorage.setItem(key, "A-key");
  localStorage.setItem(collapse, "1");
  useMachinePaneStore.getState().bindAccount("A");
  useMachinePaneStore.getState().setHidden("A-thread", true);
  const saved = localStorage.getItem("nyxid.machine-pane")!;
  // A new page starts without a confirmed owner, then hydrates persisted preferences.
  transitionAssistantIdentity(null);
  useMachinePaneStore.setState({ ownerUserId: null, hidden: [] });
  localStorage.setItem("nyxid.machine-pane", saved);
  await useMachinePaneStore.persist.rehydrate();
  bindAccountResourcePreferences(owner);
  transitionAssistantIdentity(owner);
  useMachinePaneStore.getState().bindAccount(owner);
  expect(localStorage.getItem(key)).toBe(owner === "A" ? "A-key" : null);
  expect(localStorage.getItem(collapse)).toBe(owner === "A" ? "1" : null);
  expect(useMachinePaneStore.getState().hidden).toEqual(owner === "A" ? ["A-thread"] : []);
});

it.each(["B", null])("clears resource preferences through the auth transition A -> %s", async (next) => {
  const { useAuthStore } = await import("@/stores/auth-store");
  const person = (id: string) => ({ id }) as import("@/types/api").User;
  useAuthStore.getState().setUser(person("A"));
  bindAccountResourcePreferences("A");
  localStorage.setItem(key, "A-key");
  localStorage.setItem(collapse, "1");
  useMachinePaneStore.getState().bindAccount("A");
  useMachinePaneStore.getState().setHidden("A-thread", true);
  useAuthStore.getState().setUser(next ? person(next) : null);
  expect(localStorage.getItem(key)).toBeNull();
  expect(localStorage.getItem(collapse)).toBeNull();
  expect(useMachinePaneStore.getState().hidden).toEqual([]);
  expect(localStorage.getItem("nyxid.machine-pane")).not.toContain("A-thread");
});
