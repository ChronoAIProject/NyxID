import { expect, test, type Page } from "@playwright/test";

const LINK_ID = "11111111-1111-4111-8111-111111111111";
const OTHER_ID = "22222222-2222-4222-8222-222222222222";

async function fixture(
  page: Page,
  baseURL: string,
  options: { enabled?: boolean; acknowledge?: boolean } = {},
) {
  const state = {
    userId: "33333333-3333-4333-8333-333333333333",
    authenticated: true,
    onboarding: false,
    completedOnboarding: 0,
    status: "pending",
    nextLinkId: LINK_ID,
    responseId: null as string | null,
    lastError: null as string | null,
    connectedService: {
      id: "44444444-4444-4444-8444-444444444444",
      slug: "google-work",
    } as { id: string; slug: string } | null,
    creates: [] as Record<string, unknown>[],
    statusReads: 0,
    cancels: 0,
  };
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.origin === "https://hosted.example")
      return route.fulfill({
        contentType: "text/html",
        body: "<p>Hosted setup fixture</p>",
      });
    if (url.origin !== new URL(baseURL).origin) return route.abort();
    if (!url.pathname.startsWith("/api/v1/")) return route.continue();
    const json = (body: unknown, status = 200) =>
      route.fulfill({
        status,
        contentType: "application/json",
        body: JSON.stringify(body),
      });
    if (url.pathname === "/api/v1/users/me" && !state.authenticated)
      return json({ message: "Sign in", error_code: 1000 }, 401);
    if (url.pathname === "/api/v1/users/me")
      return json({
        id: state.userId,
        email: "fixture@example.test",
        display_name: "Fixture",
        avatar_url: null,
        email_verified: true,
        mfa_enabled: false,
        is_admin: false,
        is_active: true,
        created_at: "2026-09-10T00:00:00Z",
        profile_config: {
          onboarding: {
            ai_services_completed_at: state.onboarding
              ? null
              : "2026-09-10T00:00:00Z",
          },
        },
      });
    if (url.pathname === "/api/v1/users/me/onboarding/complete") {
      state.completedOnboarding++;
      state.onboarding = false;
      return json({ ai_services_completed_at: "2026-09-10T00:00:00Z" });
    }
    if (url.pathname === "/api/v1/public/config")
      return json({
        social_providers: [],
        email_auth_enabled: true,
        invite_code_required: false,
        oauth_return_routes_enabled: options.enabled ?? true,
      });
    if (url.pathname === "/api/v1/keys") return json({ keys: [] });
    if (url.pathname === "/api/v1/api-keys") return json({ keys: [] });
    if (url.pathname === "/api/v1/developer/oauth-clients")
      return json({ clients: [] });
    if (url.pathname === "/api/v1/nodes") return json({ nodes: [] });
    if (url.pathname === "/api/v1/orgs") return json({ orgs: [] });
    if (url.pathname === "/api/v1/connect-links") {
      state.creates.push(
        route.request().postDataJSON() as Record<string, unknown>,
      );
      return json({
        id: state.nextLinkId,
        connect_url: "https://hosted.example/connect/nyx_clk_fixture",
        expires_at: "2099-09-10T00:15:00Z",
        ...(options.acknowledge === false
          ? {}
          : { callback_url: `${baseURL}/temp?step=workspace` }),
      });
    }
    if (/^\/api\/v1\/connect-links\/[^/]+\/cancel$/.test(url.pathname)) {
      state.cancels++;
      state.status = "cancelled";
    }
    if (/^\/api\/v1\/connect-links\/[^/]+(?:\/cancel)?$/.test(url.pathname)) {
      state.statusReads++;
      return json({
        id: state.responseId ?? url.pathname.split("/")[4],
        status: state.status,
        service_name: "Google API",
        service_slug: "api-google",
        expires_at: "2099-09-10T00:15:00Z",
        connected_service:
          state.status === "completed" ? state.connectedService : null,
        last_error: state.lastError,
      });
    }
    return json(
      {
        error: "fixture_unhandled",
        error_code: -1,
        message: "No real API request allowed",
      },
      404,
    );
  });
  return state;
}

test("configured return survives navigation and reload and verifies server status", async ({
  page,
  baseURL,
}) => {
  const state = await fixture(page, baseURL!);
  await page.goto("/temp");
  await page.getByRole("button", { name: "Prepare connection" }).click();
  await expect(
    page.getByRole("link", { name: "Open Google setup" }),
  ).toBeVisible();
  expect(state.creates).toEqual([
    { service_slug: "api-google", return_page: "local-poc", expires_in: 900 },
  ]);
  await expect(
    page.getByText(`${baseURL}/temp?step=workspace`, { exact: false }),
  ).toBeVisible();
  await page.getByRole("link", { name: "Open Google setup" }).click();
  await expect(page.getByText("Hosted setup fixture")).toBeVisible();
  await page.goto(
    `/temp?step=workspace&status=completed&connect_link_id=${LINK_ID}`,
  );
  await expect(
    page.getByText("Waiting for authorization for this request."),
  ).toBeVisible();
  await expect(
    page.getByText(/Google connection confirmed by NyxID/),
  ).toHaveCount(0);
  state.status = "completed";
  await page
    .getByRole("button", { name: "Refresh result", exact: true })
    .click();
  await expect(
    page.getByText(/Google connection confirmed by NyxID/),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByText(/Google connection confirmed by NyxID/),
  ).toBeVisible();
  await page.getByRole("button", { name: "Start another request" }).click();
  await page.getByLabel("Return page").selectOption("default");
  await page.getByRole("button", { name: "Prepare connection" }).click();
  await expect.poll(() => state.creates.length).toBe(2);
  expect(state.creates[1]?.return_page).toBe("default");
});

test("a mismatched or duplicate callback and another account cannot accept a saved result", async ({
  page,
  baseURL,
}) => {
  const state = await fixture(page, baseURL!);
  await page.goto("/temp");
  await page.getByRole("button", { name: "Prepare connection" }).click();
  await expect(
    page.getByRole("link", { name: "Open Google setup" }),
  ).toBeVisible();
  state.status = "completed";
  for (const query of [
    `connect_link_id=${OTHER_ID}`,
    `connect_link_id=${LINK_ID}&connect_link_id=${OTHER_ID}`,
  ]) {
    await page.goto(`/temp?status=completed&${query}`);
    await expect(
      page.getByText(/does not match a request saved for your account/),
    ).toBeVisible();
    await expect(
      page.getByText(/Google connection confirmed by NyxID/),
    ).toHaveCount(0);
  }
  state.userId = OTHER_ID;
  const reads = state.statusReads;
  await page.goto(`/temp?status=completed&connect_link_id=${LINK_ID}`);
  await expect(
    page.getByText(/does not match a request saved for your account/),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Prepare connection" }),
  ).toBeVisible();
  expect(state.statusReads).toBe(reads);
});

test("an older backend response blocks setup and allows cancellation", async ({
  page,
  baseURL,
}) => {
  const state = await fixture(page, baseURL!, { acknowledge: false });
  await page.goto("/temp");
  await page.getByRole("button", { name: "Prepare connection" }).click();
  await expect(
    page.getByText(/backend did not confirm a configured destination/),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Open Google setup" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Cancel request" }).click();
  await expect(page.getByText("Connection request cancelled.")).toBeVisible();
  expect(state.cancels).toBe(1);
});

test("a backend without the feature cannot create a named request", async ({
  page,
  baseURL,
}) => {
  const state = await fixture(page, baseURL!, { enabled: false });
  await page.goto("/temp");
  await expect(
    page.getByRole("button", { name: "Prepare connection" }),
  ).toBeDisabled();
  expect(state.creates).toHaveLength(0);
});

test("provider denial remains retryable and a mismatched API result is rejected", async ({
  page,
  baseURL,
}) => {
  const state = await fixture(page, baseURL!);
  await page.goto("/temp");
  await page.getByRole("button", { name: "Prepare connection" }).click();
  await expect(
    page.getByRole("link", { name: "Open Google setup" }),
  ).toBeVisible();
  state.lastError = "provider_access_denied";
  await page
    .getByRole("button", { name: "Refresh result", exact: true })
    .click();
  await expect(page.getByText(/provider_access_denied/)).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Open Google setup" }),
  ).toBeVisible();
  state.responseId = OTHER_ID;
  for (const result of ["completed", "cancelled", "expired"] as const) {
    state.status = result;
    await page
      .getByRole("button", { name: "Refresh result", exact: true })
      .click();
    await expect(page.getByText(/backend result does not match/)).toBeVisible();
    await expect(
      page.getByText(/Google connection confirmed by NyxID/),
    ).toHaveCount(0);
    await expect(page.getByText("Connection request cancelled.")).toHaveCount(
      0,
    );
    await expect(page.getByText(/Connection request expired/)).toHaveCount(0);
    await expect(page.getByText(/provider_access_denied/)).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Start another request" }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Cancel request" }),
    ).toBeVisible();
  }
});

for (const [path, returnPage] of [
  ["/dashboard", "dashboard"],
  ["/ai-setup", "onboarding"],
] as const) {
  test(`${path} creates and resumes its configured return`, async ({
    page,
    baseURL,
  }) => {
    const state = await fixture(page, baseURL!);
    await page.goto(path);
    await page
      .getByRole("button", { name: "Connect Google", exact: true })
      .click();
    await expect(
      page.getByRole("link", { name: "Open Google setup" }),
    ).toBeVisible();
    expect(state.creates[0]?.return_page).toBe(returnPage);
    state.status = "completed";
    await page.goto(`${path}?connect_link_id=${LINK_ID}&status=completed`);
    await expect(
      page.getByText(/Google connection confirmed by NyxID/),
    ).toBeVisible();
    await expect(
      page.getByRole("link", { name: "View connected service" }),
    ).toHaveAttribute("href", /44444444-4444-4444-8444-444444444444/);
  });

  test(`${path} preserves a signed-out return through login`, async ({
    page,
    baseURL,
  }) => {
    const state = await fixture(page, baseURL!);
    state.authenticated = false;
    const destination = `${baseURL}${path}?connect_link_id=${LINK_ID}&status=completed`;
    await page.goto(destination);
    await expect(page).toHaveURL(/\/login\?/);
    expect(new URL(page.url()).searchParams.get("return_to")).toBe(destination);
  });
}

test("the local POC preserves callback fields through its login link", async ({
  page,
  baseURL,
}) => {
  const state = await fixture(page, baseURL!);
  state.authenticated = false;
  const destination = `${baseURL}/temp?step=workspace&connect_link_id=${LINK_ID}&status=completed`;
  await page.goto(destination);
  await page.getByRole("link", { name: "Use the login page" }).click();
  await expect(page).toHaveURL(/\/login\?/);
  expect(new URL(page.url()).searchParams.get("return_to")).toBe(destination);
});

test("a fallback page resumes the saved attempt without overwriting another context", async ({
  page,
  baseURL,
}) => {
  const state = await fixture(page, baseURL!);
  await page.goto("/dashboard");
  await page
    .getByRole("button", { name: "Connect Google", exact: true })
    .click();
  await expect(
    page.getByRole("link", { name: "Open Google setup" }),
  ).toBeVisible();
  state.nextLinkId = OTHER_ID;
  await page.goto("/ai-setup");
  await page
    .getByRole("button", { name: "Connect Google", exact: true })
    .click();
  await expect(
    page.getByRole("link", { name: "Open Google setup" }),
  ).toBeVisible();
  state.status = "completed";
  await page.goto(`/dashboard?connect_link_id=${OTHER_ID}&status=completed`);
  await expect(
    page.getByText(/Google connection confirmed by NyxID/),
  ).toBeVisible();
  await page.getByRole("button", { name: "Start another request" }).click();
  state.status = "pending";
  await page.reload();
  await expect(
    page.getByText("Waiting for authorization for this request."),
  ).toBeVisible();
  expect(state.creates.map((input) => input.return_page)).toEqual([
    "dashboard",
    "onboarding",
  ]);
});

test("first-run onboarding advances only after the saved connection is verified", async ({
  page,
  baseURL,
}) => {
  const state = await fixture(page, baseURL!);
  state.onboarding = true;
  await page.goto("/dashboard");
  await page
    .getByRole("button", { name: "Connect Google", exact: true })
    .click();
  await expect(
    page.getByRole("link", { name: "Open Google setup" }),
  ).toBeVisible();
  expect(state.creates[0]?.return_page).toBe("onboarding");
  await page.goto(`/ai-setup?connect_link_id=${LINK_ID}&status=completed`);
  await expect(
    page.getByText("Waiting for authorization for this request."),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Continue onboarding" }),
  ).toHaveCount(0);
  expect(state.completedOnboarding).toBe(0);
  state.status = "completed";
  await page
    .getByRole("button", { name: "Refresh result", exact: true })
    .click();
  await page.getByRole("button", { name: "Continue onboarding" }).click();
  await expect(
    page.getByRole("heading", { name: "AI Setup Guide" }),
  ).toBeVisible();
  expect(state.completedOnboarding).toBe(1);
});

test("unavailable attempt storage blocks setup but preserves cancellation", async ({
  page,
  baseURL,
}) => {
  await page.addInitScript(() => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key: string, value: string) {
      if (key.startsWith("nyxid:configured-connect:"))
        throw new Error("Storage unavailable");
      original.call(this, key, value);
    };
  });
  const state = await fixture(page, baseURL!);
  await page.goto("/temp");
  await page.getByRole("button", { name: "Prepare connection" }).click();
  await expect(
    page.getByText(/cannot save the connection request/),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Open Google setup" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Cancel request" }).click();
  await expect(page.getByText("Connection request cancelled.")).toBeVisible();
  await page.getByRole("button", { name: "Start another request" }).click();
  await expect(
    page.getByRole("button", { name: "Prepare connection" }),
  ).toBeVisible();
  expect(state.cancels).toBe(1);
});

test("a completed result without its service cannot advance onboarding", async ({
  page,
  baseURL,
}) => {
  const state = await fixture(page, baseURL!);
  state.onboarding = true;
  await page.goto("/dashboard");
  await page
    .getByRole("button", { name: "Connect Google", exact: true })
    .click();
  await expect(
    page.getByRole("link", { name: "Open Google setup" }),
  ).toBeVisible();
  state.status = "completed";
  state.connectedService = null;
  await page
    .getByRole("button", { name: "Refresh result", exact: true })
    .click();
  await expect(page.getByText(/backend result does not match/)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Continue onboarding" }),
  ).toHaveCount(0);
  expect(state.completedOnboarding).toBe(0);
});
