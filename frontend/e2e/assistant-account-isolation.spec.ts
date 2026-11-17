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
  profileChecks: number;
  holdProfile: boolean;
  releaseProfile?: () => void;
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

// The real app runs against independent local HTTP account fixtures.
async function installAccountFixture(page: Page) {
  await page.addInitScript(() => {
    globalThis.__nyxidAssistantHttpFaults = { nyxagentEnabled: true };
    const fixture: AccountFixture = {
      owner: "A",
      conversations: { A: "", B: "" },
      holdHistory: false,
      held: false,
      violations: [],
      documentToken: crypto.randomUUID(),
      profileChecks: 0,
      holdProfile: false,
    };
    const serverOwnerKey = "account-fixture-server-owner";
    if (!localStorage.getItem(serverOwnerKey)) localStorage.setItem(serverOwnerKey, "A");
    Object.defineProperty(fixture, "owner", {
      get: () => localStorage.getItem(serverOwnerKey) as "A" | "B",
      set: (owner: string) => localStorage.setItem(serverOwnerKey, owner),
    });
    (window as AppWindow).__accountFixture = fixture;
    const profileReleases: (() => void)[] = [];
    fixture.releaseProfile = () => profileReleases.splice(0).forEach((release) => release());
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
              message.text = `${owner} recognisable private conversation` + ((window as unknown as { __longTranscript?: boolean }).__longTranscript
                ? "\n\n" + Array.from({ length: 80 }, (_, index) => `Transcript line ${index + 1}.`).join("\n\n") : "");
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
      if (endpoint === "/auth/logout") { localStorage.setItem(serverOwnerKey, "signed-out"); return new Response(null, { status: 204 }); }
      if (endpoint === "/auth/login") {
        const credentials = JSON.parse(String(init?.body)) as { email: string };
        fixture.owner = credentials.email.startsWith("B") ? "B" : "A";
        return Response.json({});
      }
      if (endpoint === "/users/me") {
        fixture.profileChecks++;
        if (fixture.holdProfile) await new Promise<void>((resolve) => { profileReleases.push(resolve); });
        if (localStorage.getItem(serverOwnerKey) === "signed-out") return Response.json({ error: "unauthorized", error_code: 1001 }, { status: 401 });
        body = {
          ...(body as object),
          id: fixture.owner,
          display_name: `Account ${fixture.owner}`,
          email: `${fixture.owner}@example.test`,
        };
      }
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
}

test("isolates A -> B -> A, late A history, and SPA back/forward without reloading", async ({ page }) => {
  await installAccountFixture(page);
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

test("three minutes of background verification preserve focus, draft, scroll and the open account panel", async ({ page }) => {
  await page.addInitScript(() => { (window as unknown as { __longTranscript: boolean }).__longTranscript = true; });
  await installAccountFixture(page);
  await page.clock.install();
  await page.goto("/assistant", { waitUntil: "domcontentloaded" });
  await expect.poll(() => page.evaluate(() => (window as AppWindow).__nyxApp?.authStore.getState().user?.id)).toBe("A");
  const id = await page.evaluate(() => (window as AppWindow).__accountFixture.conversations.A);
  await navigate(page, id);
  await expect(page.getByText(/^A recognisable private conversation/).first()).toBeVisible();
  const composer = page.locator("[data-composer-textbox]");
  await composer.fill("keep this focused private draft");
  await composer.focus();
  // Settle the initial transcript layout and mark scrolling away from the tail.
  await page.clock.runFor(2_000);
  await page.evaluate(() => {
    const composer = document.querySelector("[data-composer-textbox]")!;
    const violations: string[] = [];
    const check = () => {
      if (document.documentElement.hasAttribute("data-account-verifying") || document.documentElement.hasAttribute("data-account-transition") || getComputedStyle(document.body).visibility === "hidden") violations.push("hidden");
      if (!composer.isConnected) violations.push("remounted");
    };
    new MutationObserver(check).observe(document, { subtree: true, childList: true, attributes: true });
    Object.assign(window, { __backgroundProbe: { composer, violations, user: (window as AppWindow).__nyxApp.authStore.getState().user } });
    (window as AppWindow).__accountFixture.holdProfile = true;
  });
  await page.evaluate(() => {
    const store = (window as AppWindow).__nyxApp.authStore;
    const verify = store.getState().verifySession;
    Object.assign(window, { __completedVerifications: 0 });
    store.setState({ verifySession: () => {
      const pending = verify();
      void pending.then(() => { (window as unknown as { __completedVerifications: number }).__completedVerifications++; });
      return pending;
    } });
  });
  for (let minute = 0; minute < 3; minute++) {
    if (minute === 1) {
      await page.evaluate(async () => {
        (window as AppWindow).__accountFixture.holdProfile = false;
        const app = (window as AppWindow).__nyxApp;
        await app.router.navigate({ to: "/assistant", search: { ...app.router.state.location.search, panel: "settings" } });
      });
      await expect(page.getByRole("dialog")).toBeVisible();
      // Radix traps focus in the open panel. Its editable field must stay mounted too.
      const field = page.getByRole("dialog").locator('input:not([type="hidden"]), textarea, [contenteditable="true"]').first();
      await expect(field).toBeVisible();
      await field.fill("unsaved account panel input");
      await field.focus();
      await page.clock.runFor(500);
      await page.evaluate(() => { (window as AppWindow).__accountFixture.holdProfile = true; });
    }
    await page.evaluate(() => {
      const scroll = document.querySelector("[data-chat-transcript]")!;
      scroll.scrollTop = 100;
      scroll.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await page.clock.runFor(200);
    await page.evaluate(() => {
      const scroll = document.querySelector("[data-chat-transcript]")!;
      if (!scroll.scrollTop) throw new Error("Transcript did not scroll");
      Object.assign((window as unknown as { __backgroundProbe: object }).__backgroundProbe, {
        scroll, top: scroll.scrollTop, focus: document.activeElement, panel: document.querySelector('[role="dialog"]'),
      });
    });
    const checks = await page.evaluate(() => (window as AppWindow).__accountFixture.profileChecks);
    await page.clock.runFor(60_000);
    // The first interval may precede the boot-confirmation + 60s threshold.
    if (await page.evaluate(() => (window as AppWindow).__accountFixture.profileChecks) === checks)
      await page.clock.runFor(60_000);
    await expect.poll(() => page.evaluate(() => ({ checks: (window as AppWindow).__accountFixture.profileChecks, visible: document.visibilityState, completed: (window as unknown as { __completedVerifications: number }).__completedVerifications }))).toEqual({ checks: checks + 1, visible: "visible", completed: minute });
    // The actual apiClient must complete while /users/me is still delayed.
    await page.evaluate(async () => {
      const path = "/src/lib/api-client.ts";
      const { apiClient } = await import(path);
      await apiClient("/public/config");
      (window as AppWindow).__accountFixture.releaseProfile!();
    });
    await expect.poll(() => page.evaluate(() => (window as unknown as { __completedVerifications: number }).__completedVerifications)).toBe(minute + 1);
    await page.evaluate(() => {
      const probe = (window as unknown as { __backgroundProbe: { panel: Element | null; composer: Element; focus: Element; scroll: Element; top: number; violations: string[]; user: unknown } }).__backgroundProbe;
      if (probe.violations.length) throw new Error(probe.violations.join(","));
      if (document.activeElement !== probe.focus) throw new Error("Focus changed");
      if (probe.scroll.scrollTop !== probe.top) throw new Error(`Transcript scroll changed: ${probe.top} -> ${probe.scroll.scrollTop}`);
      if (probe.panel && !probe.panel.isConnected) throw new Error("Panel remounted");
      if (!probe.composer.isConnected) throw new Error("Composer remounted");
      if ((window as AppWindow).__nyxApp.authStore.getState().user !== probe.user) throw new Error("Unchanged user was replaced");
    });
  }
  await expect(composer).toHaveText("keep this focused private draft");
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByRole("dialog").locator('input:not([type="hidden"]), textarea, [contenteditable="true"]').first()).toHaveValue("unsaved account panel input");
});

test("real cross-tab app login broadcasts A -> B -> A, then logout clears A without reload", async ({ page, context }) => {
  await installAccountFixture(page);
  await page.goto("/assistant", { waitUntil: "domcontentloaded" });
  await expect.poll(() => page.evaluate(() => (window as AppWindow).__nyxApp?.authStore.getState().user?.id)).toBe("A");
  const ids = await page.evaluate(() => (window as AppWindow).__accountFixture.conversations);
  await navigate(page, ids.A);
  await expect(page.getByText("A recognisable private conversation", { exact: true })).toBeVisible();
  const token = await page.evaluate(() => (window as AppWindow).__accountFixture.documentToken);
  const second = await context.newPage();
  await installAccountFixture(second);
  await second.goto("/assistant", { waitUntil: "domcontentloaded" });
  await expect.poll(() => second.evaluate(() => (window as AppWindow).__nyxApp?.authStore.getState().user?.id)).toBe("A");
  // Disable the storage-event transport in page 1: this case proves real BroadcastChannel.
  // Record the synchronous hide even if a quick fixture confirms within one frame.
  await page.evaluate(() => {
    window.addEventListener("storage", (event) => { if (event.key === "nyxid:account-session") event.stopImmediatePropagation(); }, { capture: true });
    Object.assign(window, { __crossTabHides: 0 });
    new MutationObserver((records) => {
      for (const record of records) if (record.attributeName === "data-account-verifying" && record.oldValue === null)
        (window as unknown as { __crossTabHides: number }).__crossTabHides++;
    }).observe(document.documentElement, { attributes: true, attributeOldValue: true });
  });
  for (const owner of ["B", "A"] as const) {
    await second.evaluate(async () => { await (window as AppWindow).__nyxApp.router.navigate({ to: "/privacy" }); });
    await expect(second).toHaveURL(/\/privacy/);
    await second.evaluate(async (id) => {
      const store = (window as AppWindow).__nyxApp.authStore;
      await store.getState().login(`${id}@example.test`, "local-fixture-password");
    }, owner);
    await expect.poll(() => page.evaluate(() => (window as AppWindow).__nyxApp.authStore.getState().user?.id)).toBe(owner);
    await navigate(page, ids[owner]);
    await expect(page.getByText(`${owner} recognisable private conversation`, { exact: true })).toBeVisible();
    await expect(page.getByText(`${owner === "A" ? "B" : "A"} recognisable private conversation`, { exact: true })).toHaveCount(0);
  }
  expect(await page.evaluate(() => (window as unknown as { __crossTabHides: number }).__crossTabHides)).toBeGreaterThanOrEqual(2);
  await second.evaluate(async () => { await (window as AppWindow).__nyxApp.authStore.getState().logout(); });
  await expect(page).toHaveURL(/\/login/);
  await expect(page.getByText("A recognisable private conversation", { exact: true })).toHaveCount(0);
  expect(await page.evaluate(() => (window as AppWindow).__nyxApp.authStore.getState().user)).toBeNull();
  expect(await page.evaluate(() => (window as AppWindow).__accountFixture.documentToken)).toBe(token);
  expect(await page.evaluate(() => (window as AppWindow).__accountFixture.violations)).toEqual([]);
});
