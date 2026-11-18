import { expect, it, vi } from "vitest";
import {
  invalidateAccountEvidence,
  settleAccountEvidence,
} from "@/lib/assistant/identity";
import { pollOAuthKeyUntilActive } from "./auth-flow-polling";

it("ends polling silently when evidence invalidates a queued completion", async () => {
  const getKey = vi.fn().mockResolvedValue({ status: "active" });
  const completeWithKey = vi.fn();
  const onTerminalFailure = vi.fn();
  const onTimeout = vi.fn();
  await pollOAuthKeyUntilActive({
    keyId: "fixture-key",
    getKey,
    completeWithKey,
    isCancelled: () => false,
    onTerminalFailure,
    onTimeout,
    sleepMs: async () => {
      invalidateAccountEvidence();
      settleAccountEvidence();
    },
  });
  expect(getKey).not.toHaveBeenCalled();
  expect(completeWithKey).not.toHaveBeenCalled();
  expect(onTerminalFailure).not.toHaveBeenCalled();
  expect(onTimeout).not.toHaveBeenCalled();
});
