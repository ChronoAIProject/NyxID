import { beforeEach, describe, expect, it } from "vitest";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { useCreditsDenialStore } from "./credits-denial-store";

const notify = (key: string, actorId: string | null = "person-a") =>
  useCreditsDenialStore.getState().notify({ key, payer: "self", actorId });

beforeEach(() => {
  transitionAssistantIdentity(null);
  transitionAssistantIdentity("person-a");
  useCreditsDenialStore.getState().reset();
});

describe("useCreditsDenialStore", () => {
  it("opens once per operation key", () => {
    notify("op:a:1");
    expect(useCreditsDenialStore.getState().current?.key).toBe("op:a:1");
    useCreditsDenialStore.getState().dismiss();
    notify("op:a:1");
    expect(useCreditsDenialStore.getState().current).toBeNull();
  });

  it("reopens for a new foreground operation", () => {
    notify("op:a:1");
    useCreditsDenialStore.getState().dismiss();
    notify("op:a:2");
    expect(useCreditsDenialStore.getState().current?.key).toBe("op:a:2");
  });

  it("keeps the open dialog when another denial arrives", () => {
    notify("op:a:1");
    notify("op:b:1");
    expect(useCreditsDenialStore.getState().current?.key).toBe("op:a:1");
  });

  it("ignores a late response dispatched by another identity", () => {
    transitionAssistantIdentity("person-b");
    notify("op:a:1", "person-a");
    expect(useCreditsDenialStore.getState().current).toBeNull();
    notify("op:a:2", null);
    expect(useCreditsDenialStore.getState().current).toBeNull();
  });

  it("resets on logout and account switch", () => {
    notify("op:a:1");
    transitionAssistantIdentity(null);
    expect(useCreditsDenialStore.getState().current).toBeNull();
    expect(useCreditsDenialStore.getState().seenKeys.size).toBe(0);
    transitionAssistantIdentity("person-a");
    notify("op:a:1");
    expect(useCreditsDenialStore.getState().current?.key).toBe("op:a:1");
  });
});
