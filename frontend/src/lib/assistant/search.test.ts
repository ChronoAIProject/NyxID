import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import { describe, expect, it } from "vitest";
import { parseAssistantSearch } from "./search";

describe("parseAssistantSearch", () => {
  it("keeps a conversation id", () => {
    expect(parseAssistantSearch({ c: "conv-1" })).toEqual({ c: "conv-1" });
  });

  it("accepts draft as a boolean and as the string a typed URL produces", () => {
    expect(parseAssistantSearch({ draft: true })).toEqual({ draft: true });
    expect(parseAssistantSearch({ draft: "true" })).toEqual({ draft: true });
  });

  it("preserves only the dev HTTP fixture marker", () => {
    expect(parseAssistantSearch({ mock: "1" })).toEqual({ mock: 1 });
    expect(parseAssistantSearch({ mock: 1 })).toEqual({ mock: 1 });
    expect(parseAssistantSearch({ mock: "true" })).toEqual({});
  });

  it("drops junk rather than letting it reach the page", () => {
    expect(parseAssistantSearch({ c: 42, draft: "yes", other: "x" })).toEqual(
      {},
    );
  });
});

/**
 * The page's stubbed-router tests cannot see this seam: whether a real
 * router actually round-trips `{ draft: true }` through the URL and back.
 * A param silently dropped by `validateSearch`, or serialized to something
 * `parseAssistantSearch` no longer recognises, would leave "New chat"
 * navigating to a URL that reads as "no draft" — the exact dead-button
 * symptom this change fixes, reintroduced one layer down.
 */
function buildAssistantRouter(initialUrl: string) {
  const rootRoute = createRootRoute();
  const assistantRoute = createRoute({
    path: "/assistant",
    getParentRoute: () => rootRoute,
    validateSearch: parseAssistantSearch,
    component: () => null,
  });
  return createRouter({
    routeTree: rootRoute.addChildren([assistantRoute]),
    history: createMemoryHistory({ initialEntries: [initialUrl] }),
  });
}

describe("/assistant search round-trip through a real router", () => {
  it("survives a navigate to the draft state", async () => {
    const router = buildAssistantRouter("/assistant");
    await router.load();

    await router.navigate({ to: "/assistant", search: { draft: true } });
    await router.invalidate();

    const search = router.state.location.search as Record<string, unknown>;
    expect(parseAssistantSearch(search).draft).toBe(true);
    expect(router.state.location.searchStr).toContain("draft");
  });

  it("swaps the draft for a conversation id", async () => {
    const router = buildAssistantRouter("/assistant?draft=true");
    await router.load();
    expect(
      parseAssistantSearch(
        router.state.location.search as Record<string, unknown>,
      ).draft,
    ).toBe(true);

    await router.navigate({
      to: "/assistant",
      search: { c: "conv-new" },
      replace: true,
    });
    await router.invalidate();

    const search = parseAssistantSearch(
      router.state.location.search as Record<string, unknown>,
    );
    expect(search.c).toBe("conv-new");
    expect(search.draft).toBeUndefined();
  });

  it("round-trips the numeric mock marker without quoting it", async () => {
    const router = buildAssistantRouter("/assistant?mock=1");
    await router.load();

    await router.navigate({
      to: "/assistant",
      search: { c: "nyxid-chat-mock-1", mock: 1 },
      replace: true,
    });
    await router.invalidate();

    expect(router.state.location.searchStr).toContain("mock=1");
    expect(router.state.location.searchStr).not.toContain("%22");
    expect(
      parseAssistantSearch(
        router.state.location.search as Record<string, unknown>,
      ),
    ).toEqual({ c: "nyxid-chat-mock-1", mock: 1 });
  });
});
