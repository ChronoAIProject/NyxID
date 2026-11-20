import { expect, test, type Page } from "@playwright/test";
import type { QueryClient } from "@tanstack/react-query";
import type { router } from "../src/router";
import type { useAuthStore } from "../src/stores/auth-store";

// Full Chromium can restore a live document through the back/forward cache.
test.use({ channel: "chromium", launchOptions: { ignoreDefaultArgs: ["--disable-back-forward-cache"] } });

type Owner = "A" | "B";
type AppWindow = Window & {
  __nyxApp: { authStore: typeof useAuthStore; router: typeof router };
  __nyxQueryClient: QueryClient;
  fixture: {
    ids: Record<Owner, string>;
    holdHistory: boolean;
    release?: () => void;
    token: string;
    exposed: string[];
  };
};

test.beforeEach(async ({ context }) => {
  await context.addInitScript(() => {
    if (location.pathname === "/robots.txt") return;
    const app = window as AppWindow;
    globalThis.__nyxidAssistantHttpFaults = { nyxagentEnabled: true };
    const fixture = app.fixture = {
      ids: { A: "", B: "" }, holdHistory: false, token: crypto.randomUUID(), exposed: [],
    } as AppWindow["fixture"];
    const owner = () => (localStorage.getItem("fixture-cookie") ?? "A") as Owner;
    const nativeFetch = window.fetch.bind(window);
    const ready = (async () => {
      const fixturePath = "/src/lib/assistant/nyxagent-http-fixtures.ts";
      const { NyxAgentHttpFixtures } = await import(fixturePath) as typeof import("../src/lib/assistant/nyxagent-http-fixtures");
      function world(id: Owner) {
        const key = "nyxagent-http-fixture-v4";
        sessionStorage.removeItem(key);
        new NyxAgentHttpFixtures();
        const state = JSON.parse(sessionStorage.getItem(key)!);
        const [threadId, row] = state.rows.find(([, row]: [string, { history: { conversation: { channel: unknown } } }]) =>
          !row.history.conversation.channel);
        fixture.ids[id] = threadId;
        for (const message of row.history.messages) {
          if (message.role === "assistant") message.text = id + " private transcript";
        }
        sessionStorage.setItem(key, JSON.stringify(state));
        return new NyxAgentHttpFixtures();
      }
      const worlds = { A: world("A"), B: world("B") };
      const fixturePage = "/src/components/assistant/assistant-http-fixture-page.tsx";
      await import(fixturePage);
      const fallback = globalThis.__nyxidAssistantHttpMock;
      globalThis.__nyxidAssistantHttpMock = async request => {
        const response = await worlds[owner()]?.handler(request);
        if (response?.status === 404 && request.endpoint.includes("/conversations/")) {
          return Response.json({ error: "not_found", message: "Conversation not found" }, { status: 404 });
        }
        if (fixture.holdHistory && request.endpoint.split("?")[0] === "/assistant/nyxagent/conversations/" + fixture.ids.A) {
          fixture.holdHistory = false;
          // Release the old response even if the application cancelled the request.
          await new Promise<void>(resolve => { fixture.release = resolve; });
        }
        return response ?? fallback?.(request);
      };
      const mockPath = "/src/lib/mock-data.ts";
      return (await import(mockPath) as typeof import("../src/lib/mock-data")).getMockResponse;
    })();
    window.fetch = async (input, init) => {
      const url = new URL(input instanceof Request ? input.url : String(input), location.origin);
      if (!url.pathname.startsWith("/api/v1/")) return nativeFetch(input, init);
      const path = url.pathname.slice("/api/v1".length) + url.search;
      const mockResponse = await ready;
      const body = mockResponse(path, init?.method ?? "GET", init?.body ? JSON.parse(String(init.body)) : undefined);
      if (path === "/auth/logout") {
        localStorage.setItem("fixture-cookie", "signed-out");
        return new Response(null, { status: 204 });
      }
      if (path === "/users/me") {
        if (localStorage.getItem("fixture-cookie") === "signed-out") return Response.json({}, { status: 401 });
        return Response.json({ ...body as object, id: owner(), email: owner() + "@example.test" });
      }
      if (path.startsWith("/remote-machines")) return Response.json({ machines: [], next_cursor: null });
      if (path === "/machines" || path.startsWith("/assistant/nyxagent/machines?")) return Response.json([]);
      return Response.json(body ?? {});
    };
    new MutationObserver(() => {
      const user = app.__nyxApp?.authStore.getState().user?.id;
      if (user !== "A" && user !== "B") return;
      const foreign = (user === "A" ? "B" : "A") + " private transcript";
      for (const p of document.querySelectorAll("main p")) {
        if (p.textContent?.includes(foreign) && p.getClientRects().length) fixture.exposed.push(foreign);
      }
    }).observe(document, { childList: true, subtree: true, attributes: true, characterData: true });
  });
});

const transcript = (page: Page, owner: Owner) => page.getByText(owner + " private transcript", { exact: true });
const missing = (page: Page) => page.getByText(/Conversation not found/i).first();
async function openThread(page: Page, owner: Owner) {
  await page.evaluate(async id => {
    const app = window as AppWindow;
    await app.__nyxApp.router.navigate({ to: "/assistant", search: { c: app.fixture.ids[id] } });
  }, owner);
  await expect(transcript(page, owner)).toBeVisible();
}

test("purges the open conversation on account switch and sign-out", async ({ page, context }) => {
  await page.goto("/assistant");
  await openThread(page, "A");
  const token = await page.evaluate(() => (window as AppWindow).fixture.token);
  const composer = page.locator("[data-composer-textbox]");
  await composer.fill("A unsent private draft");
  await page.evaluate(() => (window as AppWindow).__nyxApp.authStore.getState().checkAuth());
  await expect(composer).toHaveText("A unsent private draft");

  await page.evaluate(() => {
    const app = window as AppWindow;
    app.fixture.holdHistory = true;
    void app.__nyxQueryClient.invalidateQueries({ predicate: q => q.queryKey.includes(app.fixture.ids.A) });
  });
  await expect.poll(() => page.evaluate(() => Boolean((window as AppWindow).fixture.release))).toBe(true);
  const other = await context.newPage();
  await other.goto("/login");
  await expect.poll(() => other.evaluate(() => (window as AppWindow).__nyxApp?.authStore.getState().user?.id)).toBe("A");
  await other.evaluate(async () => {
    localStorage.setItem("fixture-cookie", "B");
    await (window as AppWindow).__nyxApp.authStore.getState().checkAuth();
  });
  await expect(missing(page)).toBeVisible();
  await page.evaluate(() => (window as AppWindow).fixture.release!());
  await openThread(page, "B");
  await expect(transcript(page, "A")).toHaveCount(0);
  await expect(composer).toBeEmpty();
  await page.goBack();
  await expect(missing(page)).toBeVisible();
  await expect(transcript(page, "A")).toHaveCount(0);
  await page.goForward();
  await expect(transcript(page, "B")).toBeVisible();

  await page.evaluate(async () => {
    localStorage.setItem("fixture-cookie", "A");
    await (window as AppWindow).__nyxApp.authStore.getState().checkAuth();
  });
  await expect(missing(page)).toBeVisible();
  await expect(transcript(page, "B")).toHaveCount(0);
  await openThread(page, "A");
  await expect(composer).toBeEmpty();
  expect(await page.evaluate(() => (window as AppWindow).fixture.token)).toBe(token);
  expect(await page.evaluate(() => (window as AppWindow).fixture.exposed)).toEqual([]);
  await other.evaluate(() => (window as AppWindow).__nyxApp.authStore.getState().logout());
  await expect(page).toHaveURL(/\/login(?:\?|$)/);
  await expect(transcript(page, "A")).toHaveCount(0);
});

test("Back keeps A hidden while the restored document verifies the cookie", async ({ page }) => {
  await page.goto("/assistant");
  await openThread(page, "A");
  await page.evaluate(() => {
    window.addEventListener("pageshow", event => {
      if (event.persisted) {
        const old = Array.from(document.querySelectorAll("main p")).find(p => p.textContent?.includes("A private transcript"));
        sessionStorage.setItem("restore-painted-A", String(Boolean(old?.getClientRects().length)));
      }
    });
  });
  await page.goto("/robots.txt");
  await page.evaluate(() => localStorage.setItem("fixture-cookie", "B"));
  await page.goBack({ waitUntil: "commit" });
  await expect(missing(page)).toBeVisible();
  expect(await page.evaluate(() => sessionStorage.getItem("restore-painted-A"))).toBe("false");
  await expect(transcript(page, "A")).toHaveCount(0);
});
