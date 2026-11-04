import { afterEach, beforeEach, expect, it } from "vitest";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { cmaSession } from "@/lib/assistant/cma-session.test-fixtures";
import { useCmaSessionStore } from "./cma-session-store";

const key = `${cmaSession.connection.id}:${cmaSession.agentId}`;

beforeEach(() => {
  transitionAssistantIdentity(null);
  useCmaSessionStore.getState().clear();
});
afterEach(() => {
  transitionAssistantIdentity(null);
  useCmaSessionStore.getState().clear();
});

it("restores only minimise preferences for the same owner", async () => {
  transitionAssistantIdentity("owner");
  useCmaSessionStore.getState().setMinimised("owner", key, true);
  await useCmaSessionStore.persist.rehydrate();
  expect(useCmaSessionStore.getState().minimised[key]).toBe(true);
  expect(JSON.parse(localStorage.getItem("nyxid.cma-session")!).state).toEqual({
    ownerId: "owner",
    minimised: { [key]: true },
  });
  useCmaSessionStore.getState().setMinimised("owner", key, false);
  expect(useCmaSessionStore.getState().minimised[key]).toBe(false);
});

it("clears on logout and fences stale account updates", () => {
  transitionAssistantIdentity("owner");
  useCmaSessionStore.getState().setMinimised("owner", key, true);
  transitionAssistantIdentity(null);
  expect(useCmaSessionStore.getState()).toMatchObject({
    ownerId: null,
    minimised: {},
  });
  transitionAssistantIdentity("other");
  useCmaSessionStore.getState().setMinimised("owner", key, true);
  expect(useCmaSessionStore.getState().minimised).toEqual({});
});

it("discards a different account's persisted preferences on authentication", async () => {
  localStorage.setItem(
    "nyxid.cma-session",
    JSON.stringify({
      version: 2,
      state: { ownerId: "owner", minimised: { [key]: true } },
    }),
  );
  await useCmaSessionStore.persist.rehydrate();
  expect(useCmaSessionStore.getState().minimised[key]).toBe(true);
  transitionAssistantIdentity("other");
  expect(useCmaSessionStore.getState()).toMatchObject({
    ownerId: null,
    minimised: {},
  });
});

it("discards legacy manual pins during migration", async () => {
  localStorage.setItem(
    "nyxid.cma-session",
    JSON.stringify({
      version: 1,
      state: { ownerId: "owner", pin: cmaSession, open: true },
    }),
  );
  await useCmaSessionStore.persist.rehydrate();
  expect(useCmaSessionStore.getState()).toMatchObject({
    ownerId: null,
    minimised: {},
  });
  expect(
    JSON.parse(localStorage.getItem("nyxid.cma-session")!).state,
  ).not.toHaveProperty("pin");
});

it("rejects malformed preferences and bounds retained session identities", async () => {
  localStorage.setItem(
    "nyxid.cma-session",
    JSON.stringify({
      version: 2,
      state: { ownerId: "owner", minimised: { "../../session": true } },
    }),
  );
  await useCmaSessionStore.persist.rehydrate();
  expect(useCmaSessionStore.getState().minimised).toEqual({});
  transitionAssistantIdentity("owner");
  for (let i = 0; i < 60; i++) {
    useCmaSessionStore
      .getState()
      .setMinimised("owner", `${cmaSession.connection.id}:agt_${i}`, true);
  }
  expect(Object.keys(useCmaSessionStore.getState().minimised)).toHaveLength(50);
});
