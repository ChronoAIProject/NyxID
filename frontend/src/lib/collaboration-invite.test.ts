import { afterEach, expect, it } from "vitest";
import {
  captureGroupInvite,
  clearGroupInvite,
  readGroupInvite,
} from "./collaboration-invite";
afterEach(() => {
  clearGroupInvite();
  history.replaceState(null, "", "/");
});
it("keeps the token across login without putting it into the return URL", () => {
  const token = "a".repeat(64);
  history.replaceState(null, "", `/assistant#group-invite=${token}`);
  captureGroupInvite();
  expect(location.href).not.toContain(token);
  expect(readGroupInvite()).toBe(token);
  history.replaceState(null, "", "/login?return_to=%2Fassistant");
  expect(readGroupInvite()).toBe(token);
  clearGroupInvite();
  expect(readGroupInvite()).toBeNull();
});
it("rejects duplicate and malformed bearer fragments", () => {
  history.replaceState(
    null,
    "",
    `/assistant#group-invite=${"b".repeat(64)}&group-invite=bad`,
  );
  captureGroupInvite();
  expect(readGroupInvite()).toBeNull();
  expect(location.hash).toBe("");
});
