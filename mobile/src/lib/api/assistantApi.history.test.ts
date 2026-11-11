import assert from "node:assert/strict";
import { ApiError } from "./ApiError";
import test from "node:test";
import { readFileSync } from "node:fs";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { HISTORY_UNAVAILABLE } from "../../features/assistant/assistantHistoryCompat";

const fixture = JSON.parse(readFileSync(new URL("../../../../frontend/src/lib/assistant/__fixtures__/retry-duplicate-history.json", import.meta.url), "utf8"));

test("2345 mobile history API sanitizes malformed history and recovers on GET without replay", async () => {
  let payload: unknown = { ...fixture, messages: [{ text: "private content" }] };
  const reads: string[] = [];
  const h = hookHarness({
    "./http": { requestJson: async (path: string, options?: { method?: string }) => {
      assert.equal(options?.method, undefined);
      reads.push(path);
      return payload;
    } },
  });
  const { assistantApi } = h.load(fileURLToPath(new URL("./assistantApi.ts", import.meta.url))) as unknown as typeof import("./assistantApi");
  await assert.rejects(assistantApi.history(fixture.conversation.id), { message: HISTORY_UNAVAILABLE });
  payload = fixture;
  const history = await assistantApi.history(fixture.conversation.id);
  assert.equal(history.conversation.active_turn?.activities[1]?.failure?.kind, "retry_duplicate_action");
  assert.equal(reads.length, 2);
  payload = { ...fixture, conversation: { ...fixture.conversation, id: `nyxa-${"2".repeat(32)}` } };
  await assert.rejects(assistantApi.history(fixture.conversation.id), { message: "Assistant conversation mismatch." });
});

for (const read of ["history", "index"] as const) {
  test(`2345 on 2365 keeps the response-error boundary before ${read} validation and GET recovery`, async () => {
    const { loadApiConsumers } = await import("./apiUrlTestHarness");
    const requests: string[] = [];
    let response = () => new Response(JSON.stringify(read === "history" ? fixture : { conversations: [fixture.conversation], next_cursor: null }));
    const { http } = loadApiConsumers({
      getApiBaseUrl: () => "https://api.test/api/v1",
      buildApiUrl: (path) => `https://api.test/api/v1${path}`,
    }, async (_url, init) => {
      requests.push(init?.method ?? "GET");
      return response();
    });
    const h = hookHarness({ "./http": http });
    const { assistantApi } = h.load(fileURLToPath(new URL("./assistantApi.ts", import.meta.url))) as unknown as typeof import("./assistantApi");
    const load = () => read === "history" ? assistantApi.history(fixture.conversation.id) : assistantApi.list();
    const valid = response;
    await load();
    for (const [status, body, message] of [
      [503, "<html>nginx private proxy page</html>", "NyxID is temporarily unavailable. Try again in a moment."],
      [502, "private upstream body", "NyxID is temporarily unavailable. Try again in a moment."],
      [200, "<html>nginx private proxy page</html>", "NyxID couldn't complete this request. Please try again."],
    ] as const) {
      response = () => new Response(body, { status });
      await assert.rejects(load(), { statusCode: status, message });
    }
    response = () => new Response(JSON.stringify({ messages: [{ text: "private validation data" }] }));
    await assert.rejects(load(), { message: HISTORY_UNAVAILABLE });
    const body = { error: "conflict", error_code: 1004, message: "Conflict: Request expired", details: { status: "expired" } };
    response = () => new Response(JSON.stringify(body), { status: 409 });
    await assert.rejects(load(), (error: unknown) => {
      assert.ok(error instanceof ApiError);
      assert.equal(error.statusCode, 409);
      assert.equal(error.errorKey, body.error);
      assert.equal(error.errorCode, body.error_code);
      assert.equal(error.message, body.message);
      assert.equal(JSON.stringify(error.details), JSON.stringify(body.details));
      return true;
    });
    response = valid;
    await load();
    assert.deepEqual(requests, Array(7).fill("GET"));
  });
}
