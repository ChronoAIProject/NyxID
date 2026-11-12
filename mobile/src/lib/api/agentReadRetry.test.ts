import assert from "node:assert/strict";
import test from "node:test";
import { QueryClient } from "@tanstack/react-query";
import { retryAgentRead, agentReadRetryDelay } from "./agentReadRetry";
import { buildApiError } from "./responseError";

test("roster GET retries transient failures, recovers and caps retries", async () => {
  const client = new QueryClient();
  let sends = 0;
  assert.equal(
    await client.fetchQuery({
      queryKey: ["agents"],
      retry: retryAgentRead,
      retryDelay: 0,
      queryFn: async () => {
        if (++sends < 3) throw buildApiError(null, 503);
        return "roster";
      },
    }),
    "roster",
  );
  assert.equal(sends, 3);
  sends = 0;
  await assert.rejects(
    client.fetchQuery({
      queryKey: ["unavailable"],
      retry: retryAgentRead,
      retryDelay: 0,
      queryFn: async () => {
        sends++;
        throw buildApiError(null, 502);
      },
    }),
  );
  assert.equal(sends, 3);
  assert.equal(agentReadRetryDelay(0), 1000);
  assert.equal(agentReadRetryDelay(1), 2000);
  for (const status of [400, 401, 403, 404, 409, 429])
    assert.equal(retryAgentRead(0, buildApiError(null, status)), false);
  assert.equal(
    retryAgentRead(
      0,
      Object.assign(new Error("cancelled"), { name: "AbortError" }),
    ),
    false,
  );
  assert.equal(retryAgentRead(0, new Error("auth_session_changed")), false);
  assert.equal(retryAgentRead(0, buildApiError(null, 0)), true);
  sends = 0;
  await assert.rejects(
    client
      .getMutationCache()
      .build(client, {
        mutationFn: async () => {
          sends++;
          throw buildApiError(null, 503);
        },
      })
      .execute(undefined),
  );
  assert.equal(sends, 1);
  client.clear();
});
