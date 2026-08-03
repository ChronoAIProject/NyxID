import { describe, expect, it } from "vitest";
import { parseAssistantReceiptState } from "./assistant-receipts";

const NOW = Date.parse("2026-08-04T00:00:00.000Z");

describe("assistant receipt schema", () => {
  it("parses a persisted blob and drops malformed child entries", () => {
    const parsed = parseAssistantReceiptState(
      {
        version: 1,
        ownerUserId: "user-a",
        receipts: {
          good: {
            placeholderId: "workflow-pending-a",
            createdAt: NOW,
            updatedAt: NOW,
          },
          bad: { placeholderId: "" },
        },
        deletionIntents: {
          good: { placeholderId: "workflow-pending-b", createdAt: NOW },
          bad: { placeholderId: 4, createdAt: NOW },
        },
      },
      NOW,
    );

    expect(Object.keys(parsed?.receipts ?? {})).toEqual(["good"]);
    expect(Object.keys(parsed?.deletionIntents ?? {})).toEqual(["good"]);
  });

  it.each([0, -1, Number.MAX_SAFE_INTEGER + 1, 1.5])(
    "rejects the unusable stateVersion %s",
    (stateVersion) => {
      const parsed = parseAssistantReceiptState(
        {
          version: 1,
          ownerUserId: "user-a",
          receipts: {
            bad: {
              placeholderId: "workflow-pending-a",
              stateVersion,
              createdAt: NOW,
              updatedAt: NOW,
            },
          },
          deletionIntents: {},
        },
        NOW,
      );
      expect(parsed?.receipts).toEqual({});
    },
  );

  it("drops invalid timestamps and clamps timestamps far in the future", () => {
    const parsed = parseAssistantReceiptState(
      {
        version: 1,
        ownerUserId: "user-a",
        receipts: {
          invalid: {
            placeholderId: "workflow-pending-invalid",
            createdAt: -1,
            updatedAt: NOW,
          },
          future: {
            placeholderId: "workflow-pending-future",
            createdAt: NOW + 10 * 60_000,
            updatedAt: NOW + 10 * 60_000,
          },
        },
        deletionIntents: {},
      },
      NOW,
    );

    expect(parsed?.receipts.invalid).toBeUndefined();
    expect(parsed?.receipts.future).toMatchObject({
      createdAt: NOW,
      updatedAt: NOW,
    });
  });
});
