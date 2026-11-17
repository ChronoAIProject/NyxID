import { expect, test, type Page } from "@playwright/test";
import type { useAuthStore } from "../src/stores/auth-store";
import type { router } from "../src/router";

interface AccountFixture {
  owner: "A" | "B";
  conversations: Record<"A" | "B", string>;
  holdHistory: boolean;
  held: boolean;
  release?: () => void;
  violations: string[];
  documentToken: string;
}
type AppWindow = Window & {
  __accountFixture: AccountFixture;
  __nyxApp: { authStore: typeof useAuthStore; router: typeof router };
  __nyxQueryClient: import("@tanstack/react-query").QueryClient;
};

async function navigate(page: Page, id: string) {
  await page.evaluate(async (c) => {
    await (window as AppWindow).__nyxApp.router.navigate({
      to: "/assistant",
      search: { c },
    });
  }, id);
}

// The real app and transports run in Chromium. Only the HTTP boundary has local,
// independently owned server fixtures; /users/me always reads the active account.
test("isolates A -> B -> A, late A history, and SPA back/forward without reloading", async ({
  page,
}) => {
  await page.addInitScript(() => {
    globalThis.__nyxidAssistantHttpFaults = { nyxagentEnabled: true };
    const fixture: AccountFixture = {
      owner: "A",
      conversations: { A: "", B: "" },
      holdHistory: false,
      held: false,
      violations: [],
      documentToken: crypto.randomUUID(),
    };
    (window as AppWindow).__accountFixture = fixture;
    const nativeFetch = window.fetch.bind(window);
    const ready = (async () => {
      const fixturePath = "/src/lib/assistant/nyxagent-http-fixtures.ts";
      const mockPath = "/src/lib/mock-data.ts";
      const { NyxAgentHttpFixtures } = (await import(
        fixturePath
      )) as typeof import("../src/lib/assistant/nyxagent-http-fixtures");
      const { getMockResponse } = (await import(
        mockPath
      )) as typeof import("../src/lib/mock-data");
      const worlds = {} as Record<
        "A" | "B",
        InstanceType<typeof NyxAgentHttpFixtures>
      >;
      for (const owner of ["A", "B"] as const) {
        sessionStorage.removeItem("nyxagent-http-fixture-v4");
        new NyxAgentHttpFixtures();
        const state = JSON.parse(
          sessionStorage.getItem("nyxagent-http-fixture-v4")!,
        );
        const row = state.rows.find(
          ([, value]: [
            string,
            { history: { conversation: { channel: unknown } } },
          ]) => !value.history.conversation.channel,
        );
        fixture.conversations[owner] = row[0];
        row[1].history.messages.forEach(
          (message: { role: string; text: string }) => {
            if (message.role === "assistant")
              message.text = `${owner} recognisable private conversation`;
          },
        );
        sessionStorage.setItem(
          "nyxagent-http-fixture-v4",
          JSON.stringify(state),
        );
        worlds[owner] = new NyxAgentHttpFixtures();
      }
      const pageFixturePath =
        "/src/components/assistant/assistant-http-fixture-page.tsx";
      await import(pageFixturePath);
      const fallback = globalThis.__nyxidAssistantHttpMock;
      globalThis.__nyxidAssistantHttpMock = async (request) => {
        const owner = fixture.owner;
        const response = await worlds[owner].handler(request);
        if (
          response?.status === 404 &&
          request.endpoint.includes("/conversations/")
        )
          return Response.json(
            {
              error: "not_found",
              error_code: 1000,
              message: "Conversation not found",
            },
            { status: 404 },
          );
        if (
          fixture.holdHistory &&
          owner === "A" &&
          request.endpoint.split("?")[0] ===
            `/assistant/nyxagent/conversations/${fixture.conversations.A}`
        ) {
          fixture.holdHistory = false;
          fixture.held = true;
          // Deliberately ignore the AbortSignal to exercise the response fence.
          await new Promise<void>((resolve) => {
            fixture.release = resolve;
          });
        }
        return response ?? fallback?.(request);
      };
      return getMockResponse;
    })();
    window.fetch = async (input, init) => {
      const url = new URL(
        input instanceof Request ? input.url : String(input),
        location.origin,
      );
      if (!url.pathname.startsWith("/api/v1/")) return nativeFetch(input, init);
      const getMockResponse = await ready;
      const endpoint = url.pathname.slice("/api/v1".length) + url.search;
      let body = getMockResponse(
        endpoint,
        init?.method ?? "GET",
        init?.body ? JSON.parse(String(init.body)) : undefined,
      );
      if (endpoint === "/users/me")
        body = {
          ...(body as object),
          id: fixture.owner,
          display_name: `Account ${fixture.owner}`,
          email: `${fixture.owner}@example.test`,
        };
      if (
        endpoint === "/machines" ||
        endpoint.startsWith("/assistant/nyxagent/machines?")
      )
        body = [];
      if (endpoint.startsWith("/remote-machines"))
        body = { machines: [], next_cursor: null };
      return Response.json(body ?? {});
    };
    const inspect = () => {
      const app = (window as AppWindow).__nyxApp;
      if (!app) return;
      const owner = app.authStore.getState().user?.id;
      if (owner !== "A" && owner !== "B") return;
      const other = owner === "A" ? "B" : "A";
      for (const element of document.querySelectorAll("main p")) {
        if (
          element.textContent?.includes(
            `${other} recognisable private conversation`,
          ) &&
          getComputedStyle(element).visibility !== "hidden" &&
          element.getClientRects().length
        )
          fixture.violations.push(`${owner} saw ${other}`);
      }
    };
    new MutationObserver(inspect).observe(document, {
      subtree: true,
      childList: true,
      attributes: true,
    });
  });
  await page.goto("/assistant", { waitUntil: "domcontentloaded" });
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as AppWindow).__nyxApp?.authStore.getState().user?.id,
      ),
    )
    .toBe("A");
  const ids = await page.evaluate(
    () => (window as AppWindow).__accountFixture.conversations,
  );
  const token = await page.evaluate(
    () => (window as AppWindow).__accountFixture.documentToken,
  );
  await navigate(page, ids.A);
  await expect(
    page.getByText("A recognisable private conversation", { exact: true }),
  ).toBeVisible();

  // Same-account confirmation keeps the mounted composer draft and transcript.
  const composer = page.locator("[data-composer-textbox]");
  await composer.fill("A unsent private draft");
  await page.evaluate(async () => {
    await (window as AppWindow).__nyxApp.authStore.getState().checkAuth();
  });
  await expect(composer).toHaveText("A unsent private draft");
  await expect(
    page.getByText("A recognisable private conversation", { exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    (window as AppWindow).__accountFixture.holdHistory = true;
    void (window as AppWindow).__nyxQueryClient.invalidateQueries({
      predicate: (query) =>
        query.queryKey.includes(
          (window as AppWindow).__accountFixture.conversations.A,
        ),
    });
  });
  await expect
    .poll(() =>
      page.evaluate(() => (window as AppWindow).__accountFixture.held),
    )
    .toBe(true);

  // An out-of-band cookie change sends only an invalidation hint; /users/me is authority.
  await page.evaluate(() => {
    (window as AppWindow).__accountFixture.owner = "B";
    window.dispatchEvent(
      new StorageEvent("storage", {
        key: "nyxid:account-session",
        newValue: JSON.stringify({ userId: "B" }),
      }),
    );
    if (!document.documentElement.hasAttribute("data-account-verifying"))
      throw new Error("A was not hidden synchronously");
  });
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as AppWindow).__nyxApp.authStore.getState().user?.id,
      ),
    )
    .toBe("B");
  await expect(
    page.getByText("A recognisable private conversation", { exact: true }),
  ).toHaveCount(0);
  await expect(page.getByText(/Conversation not found/i).first()).toBeVisible();
  await page.evaluate(() => (window as AppWindow).__accountFixture.release!());
  await navigate(page, ids.B);
  await expect(
    page.getByText("B recognisable private conversation", { exact: true }),
  ).toBeVisible();
  await expect(composer).toBeEmpty();
  await page.goBack();
  await expect(page).toHaveURL(new RegExp(ids.A));
  await expect(page.getByText(/Conversation not found/i).first()).toBeVisible();
  await expect(
    page.getByText("A recognisable private conversation", { exact: true }),
  ).toHaveCount(0);
  await page.goForward();
  await expect(
    page.getByText("B recognisable private conversation", { exact: true }),
  ).toBeVisible();

  await composer.fill("B unsent private draft");
  await page.evaluate(async () => {
    (window as AppWindow).__accountFixture.owner = "A";
    await (window as AppWindow).__nyxApp.authStore.getState().checkAuth();
  });
  await expect(
    page.getByText("B recognisable private conversation", { exact: true }),
  ).toHaveCount(0);
  await expect(page.getByText(/Conversation not found/i).first()).toBeVisible();
  await navigate(page, ids.A);
  await expect(
    page.getByText("A recognisable private conversation", { exact: true }),
  ).toBeVisible();
  await expect(composer).toBeEmpty();
  await expect
    .poll(() =>
      page.evaluate(() => (window as AppWindow).__accountFixture.violations),
    )
    .toEqual([]);
  expect(
    await page.evaluate(
      () => (window as AppWindow).__accountFixture.documentToken,
    ),
  ).toBe(token);
  await expect(page.getByText(/request aborted/i)).toHaveCount(0);
});
