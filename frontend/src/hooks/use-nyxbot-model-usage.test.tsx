import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { expect, it, vi } from "vitest";
import {
  useNyxbotModelUsage,
  nyxbotModelParams,
} from "./use-nyxbot-model-usage";
import { parseCredits } from "@/lib/credits";
import { nyxbotModelUsageSchema } from "@/schemas/nyxbot-model-usage";
import { nyxbotModelFixture } from "@/test/nyxbot-model-fixture";
const mock = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: mock }));
it("preserves submicro strings/null and rejects fabricated or invalid measures", () => {
  const fixture = nyxbotModelFixture();
  expect(
    nyxbotModelUsageSchema.parse(fixture).models[1]!.usage.gross_cost,
  ).toBe("0.000000000001");
  expect(
    nyxbotModelUsageSchema.parse(fixture).models[2]!.usage.gross_cost,
  ).toBeNull();
  fixture.models[0]!.usage.requests = -1;
  expect(nyxbotModelUsageSchema.safeParse(fixture).success).toBe(false);
});
it("encodes literal model names separately from unknown and keys date/model transitions", async () => {
  const from = "2026-10-10T00:00:00Z",
    to = "2026-10-17T00:00:00Z";
  expect(nyxbotModelParams(from, to, { model: "a/b + c" }).get("model")).toBe(
    "a/b + c",
  );
  expect(
    nyxbotModelParams(from, to, { unknown: true }).get("unknown_model"),
  ).toBe("true");
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  mock.get.mockResolvedValueOnce(nyxbotModelFixture());
  const { result, rerender } = renderHook(
    ({ model, end }) => useNyxbotModelUsage(from, end, { model }, true),
    { wrapper, initialProps: { model: "gpt-5.4", end: to } },
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  mock.get.mockImplementationOnce(() => new Promise(() => {}));
  rerender({ model: "__unknown__", end: "2026-10-16T00:00:00Z" });
  expect(result.current.data).toBeUndefined();
  expect(result.current.isPending).toBe(true);
  await waitFor(() =>
    expect(mock.get).toHaveBeenLastCalledWith(
      expect.stringContaining("model=__unknown__"),
    ),
  );
  client.clear();
});

it("HTTP fixture period, daily, model and account totals reconcile after every filter", () => {
  for (const filter of [
    undefined,
    "claude-sonnet-4-5",
    "gpt-5.4",
    null,
    "no-such-model",
  ]) {
    const data = nyxbotModelFixture(undefined, undefined, filter);
    expect(data.daily.reduce((n, row) => n + row.usage.requests, 0)).toBe(
      data.totals.requests,
    );
    expect(data.models.reduce((n, row) => n + row.usage.total_tokens, 0)).toBe(
      data.totals.total_tokens,
    );
    expect(data.accounts.reduce((n, row) => n + row.usage.events, 0)).toBe(
      data.totals.events,
    );
    for (const row of data.models) {
      const known = [
        row.usage.wallet_cost,
        row.usage.grant_cost,
        row.usage.allowance_cost,
      ];
      if (row.usage.gross_cost !== null && known.every((v) => v !== null)) {
        expect(known.reduce((n, v) => n + parseCredits(v!), 0n)).toBe(
          parseCredits(row.usage.gross_cost),
        );
      }
    }
    if (filter === "no-such-model") expect(data.totals.events).toBe(0);
  }
});
