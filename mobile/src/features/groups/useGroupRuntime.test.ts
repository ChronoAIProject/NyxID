import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { buildApiError } from "../../lib/api/responseError";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import type { useGroupChat } from "./useGroupRuntime";

test("crew send passes through definite attachment refusals and consumes uncertain sends", async () => {
  for (const [error, expected] of [
    [buildApiError({ error: "attachment_expired", error_code: 12101, message: "Attachment expired" }, 410), "attachment"],
    [buildApiError({ error: "not_found", error_code: 1003, message: "Not found: Attachment not found or expired" }, 404), "attachment"],
    [buildApiError({ error: "not_found", error_code: 1003, message: "Not found: Group not found" }, 404), "refused"],
    [buildApiError(null, 503), "uncertain"],
  ] as const) {
    let attempts = 0;
    const h = hookHarness<ReturnType<typeof useGroupChat>>({
      "react-native": { AppState: { currentState: "active", addEventListener: () => ({ remove() {} }) } },
      "@react-navigation/native": { useIsFocused: () => true },
      "@tanstack/react-query": {
        useQuery: () => ({}),
        useQueryClient: () => ({ invalidateQueries: async () => {}, cancelQueries: async () => {} }),
      },
      "../../hooks/useNetworkStatus": { useNetworkStatus: () => ({ isConnected: true }) },
      "./useGroupsLive": { useGroupsLive: () => undefined },
      "../../lib/api/groupsApi": { groupsApi: { post: async (_groupId: string, payload: { attachment_ids: string[] }) => {
        attempts++;
        assert.equal(payload.attachment_ids.length, 1);
        throw error;
      } } },
    });
    const { useGroupChat } = h.load(
      fileURLToPath(new URL("./useGroupRuntime.ts", import.meta.url)),
    ) as typeof import("./useGroupRuntime");
    h.mount(() => useGroupChat("crew"));
    const post = h.value().post({ text: "Read this", attachment_ids: ["11111111-1111-4111-8111-111111111111"] });
    if (expected === "attachment")
      await assert.rejects(post, (cause) => cause === error);
    else
      assert.equal(await post, expected === "uncertain");
    await h.settle();
    assert.equal(h.value().sending, false);
    assert.equal(h.value().recoveryRequired, expected === "uncertain");
    assert.equal(attempts, 1);
    h.unmount();
  }
});
