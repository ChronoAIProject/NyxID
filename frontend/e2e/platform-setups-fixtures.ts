import type { Page } from "@playwright/test";
import {
  platformSetup,
  setupActor,
  setupBinding,
  setupTemplates,
} from "../src/test/platform-setup-fixture";
import type { PlatformSetup } from "../src/schemas/platform-onboarding";

export async function installSetupFixture(page: Page) {
  const state = {
    actor: { ...setupActor },
    bindings: [{ ...setupBinding }],
    drafts: [platformSetup()],
    saved: [] as PlatformSetup[],
    invalidDraft: false,
    draftError: false,
    draftErrorStatus: 0,
    adoptError: false,
    adoptionGate: Promise.resolve(),
    writes: [] as { path: string; method: string; body: unknown }[],
    unexpected: [] as string[],
    pageErrors: [] as string[],
  };
  page.on("pageerror", (error) => state.pageErrors.push(error.message));
  // index.html loads Google Fonts before the app. A live font response can
  // consume the test's whole deadline before reload even starts (#2437).
  // Keep this HTTP fixture self-contained; system fonts suffice for DOM tests.
  await page.route("**/*", (route) => {
    const url = new URL(route.request().url());
    if (url.origin === "http://localhost:4634") return route.fallback();
    if (url.origin === "https://fonts.googleapis.com")
      return route.fulfill({ contentType: "text/css", body: "" });
    state.unexpected.push(`External request: ${url.origin}${url.pathname}`);
    return route.abort("blockedbyclient");
  });
  await page.addInitScript((actor) => {
    localStorage.setItem("nyxid.language", "en");
    localStorage.setItem(
      `nyxid.telemetry_consent.v3.${actor}`,
      JSON.stringify({ asked: true, enabled: false }),
    );
  }, setupActor.id);
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.replace("/api/v1", "");
    const method = request.method();
    if (method !== "GET") {
      state.writes.push({ path, method, body: request.postDataJSON() });
    }
    if (
      method === "POST" &&
      path === `/onboarding/bindings/${setupBinding.id}/revoke`
    ) {
      state.bindings = [];
      return route.fulfill({ status: 204 });
    }
    if (method === "GET") {
      if (path === "/users/me") return route.fulfill({ json: state.actor });
      if (path === "/public/config")
        return route.fulfill({ json: { telemetry_share_analytics: false } });
      if (path === "/nodes") return route.fulfill({ json: { nodes: [] } });
      if (path === "/onboarding/templates")
        return route.fulfill({ json: setupTemplates });
      if (path === "/onboarding/setups")
        return route.fulfill({ json: state.saved });
      if (path === "/onboarding/setups/archived")
        return route.fulfill({
          json: { setups: [], next_before_id: null, next_before_ms: null },
        });
      if (path === "/onboarding/bindings")
        return route.fulfill({ json: state.bindings });
      if (path === "/onboarding/anonymous-drafts") {
        if (state.draftErrorStatus) {
          const status = state.draftErrorStatus;
          const json = status === 403
            ? { error: "forbidden", message: "Binding is unavailable" }
            : status === 404
              ? { error: "not_found", message: "Platform bot configuration not found" }
              : { error: "unavailable", message: "Service temporarily unavailable." };
          return route.fulfill({ status, json });
        }
        const binding = state.bindings.find(
          (item) =>
            item.id === url.searchParams.get("binding_id") &&
            String(item.generation) ===
              url.searchParams.get("binding_generation"),
        );
        if (state.draftError || !binding)
          return route.fulfill({
            status: 409,
            json: { error: "conflict", message: "Binding changed." },
          });
        return route.fulfill({
          json: state.invalidDraft ? [{ id: "malformed-draft" }] : state.drafts,
        });
      }
    }
    const draft = state.drafts.find(
      (item) => path === `/onboarding/setups/${item.id}/adopt`,
    );
    if (method === "POST" && draft) {
      const body = request.postDataJSON();
      await state.adoptionGate;
      const binding = state.bindings.find((row) => row.id === body.binding_id);
      if (!binding || binding.generation !== body.binding_generation)
        return route.fulfill({
          status: 403,
          json: { error: "forbidden", message: "Binding is unavailable" },
        });
      const current = state.drafts.find((row) => row.id === draft.id);
      if (!current || current.revision !== body.revision)
        return route.fulfill({
          status: 409,
          json: { error: "conflict", message: "Anonymous draft expired or changed" },
        });
      if (state.adoptError)
        return route.fulfill({
          status: 503,
          json: { error: "unavailable", message: "Service temporarily unavailable." },
        });
      state.saved.push(current);
      state.drafts = state.drafts.filter((item) => item.id !== current.id);
      return route.fulfill({ json: current });
    }
    state.unexpected.push(`${method} ${path}`);
    return route.fulfill({
      status: 501,
      json: { message: "No fixture for this request" },
    });
  });
  return state;
}
