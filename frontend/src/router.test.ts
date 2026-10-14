import { afterEach, describe, expect, it } from "vitest";
import {
  channelBotSetupRoute,
  loginAgentKeyRoute,
  loginDeviceRoute,
  loginCodeRoute,
  oauthCompleteRoute,
  router,
} from "./router";
import { isPublicPath } from "./lib/public-paths";

describe.each([loginAgentKeyRoute, loginDeviceRoute])(
  "Login approval route $fullPath",
  (route) => {
    it("is public and accepts a user code without starting a preview", () => {
      expect(isPublicPath(route.fullPath)).toBe(true);
      const validate = route.options.validateSearch;
      if (typeof validate !== "function")
        throw new Error("Expected a search validator");
      expect(validate({ user_code: "2-abcd-efgh", ignored: "value" })).toEqual({
        user_code: "2-abcd-efgh",
      });
    });

    it("accepts missing and malformed search without throwing", () => {
      const validate = route.options.validateSearch;
      if (typeof validate !== "function")
        throw new Error("Expected a search validator");
      expect(validate({})).toEqual({});
      expect(validate({ user_code: "garbage!" })).toEqual({
        user_code: "garbage!",
      });
      for (const user_code of [null, 123, ["ABCD-EFGH"], {}]) {
        expect(validate({ user_code })).toEqual({ user_code: "" });
      }
    });
  },
);

it("keeps login-code minting free of code input parameters", () => {
  const validate = loginCodeRoute.options.validateSearch;
  if (typeof validate !== "function")
    throw new Error("Expected a search validator");
  expect(validate({ user_code: "ABCD-EFGH" })).toEqual({});
});

describe("OAuth completion route registration", () => {
  it("registers the completion component outside the backend OAuth namespace", () => {
    expect(oauthCompleteRoute.fullPath).toBe("/oauth-complete");
  });
});

it("protects per-platform setup routes and accepts scalar prefill values", () => {
  expect(channelBotSetupRoute.fullPath).toBe("/channel-bots/connect/$platform");
  expect(isPublicPath("/channel-bots/connect/telegram")).toBe(false);
  const validate = channelBotSetupRoute.options.validateSearch;
  if (typeof validate !== "function")
    throw new Error("Expected a search validator");
  expect(
    validate({
      label: "a".repeat(150),
      target_org_id: ["org"],
      request_id: "invalid",
      bot_token: "private",
      future_field: "value",
      nested: { value: "ignored" },
    }),
  ).toEqual({
    label: "a".repeat(128),
    target_org_id: undefined,
    request_id: undefined,
    bot_token: "private",
    future_field: "value",
  });
});

it("mounts the admin billing page behind admin auth with a URL-driven tab", () => {
  const route = Object.values(router.routesById).find((r) => r.fullPath === "/admin/billing");
  if (!route) throw new Error("admin billing route is not registered");
  expect(isPublicPath("/admin/billing")).toBe(false);
  const validate = route.options.validateSearch;
  if (typeof validate !== "function") throw new Error("Expected a search validator");
  for (const tab of ["overview", "rates", "settings"]) expect(validate({ tab })).toEqual({ tab });
  expect(validate({ tab: "secrets" })).toEqual({});
  expect(validate({ tab: ["rates"] })).toEqual({});
});

import {
  adminToolsRoute,
  providersRedirectRoute,
  servicesRedirectRoute,
} from "./router";
import { useAuthStore } from "./stores/auth-store";
import type { User } from "./types/api";

const initialAuth = useAuthStore.getState();
afterEach(() => useAuthStore.setState(initialAuth));

const catalogUser: User = {
  id: "catalog-user",
  email: "catalog@example.test",
  display_name: "Catalog Manager",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: false,
  is_operator: true,
  role: "operator",
  is_active: true,
  created_at: "2026-10-13T00:00:00Z",
  capabilities: { manage_catalog: true },
};

for (const route of [providersRedirectRoute, servicesRedirectRoute]) {
  describe(`Catalog management guard ${route.fullPath}`, () => {
    const guard = route.options.beforeLoad!;
    const context = { location: { pathname: route.fullPath } } as never;
    it("allows Catalog Managers", async () => {
      useAuthStore.setState({ user: catalogUser });
      await expect(
        Promise.resolve().then(() => guard(context)),
      ).resolves.toBeUndefined();
    });
    it("allows admins without the capability field", async () => {
      useAuthStore.setState({
        user: {
          ...catalogUser,
          role: "admin",
          is_admin: true,
          capabilities: undefined,
        },
      });
      await expect(
        Promise.resolve().then(() => guard(context)),
      ).resolves.toBeUndefined();
    });
    it.each([
      {
        ...catalogUser,
        role: "user" as const,
        is_operator: false,
        capabilities: undefined,
      },
      { ...catalogUser, capabilities: { manage_catalog: false } },
      { ...catalogUser, capabilities: undefined },
      null,
    ])("redirects users without catalog access", async (user) => {
      useAuthStore.setState({ user });
      const result = Promise.resolve().then(() => guard(context));
      if (route === servicesRedirectRoute && user?.role === "operator") {
        await expect(result).resolves.toBeUndefined();
      } else {
        await expect(result).rejects.toMatchObject({
          options: { to: "/keys" },
        });
      }
    });
  });
}

it("keeps the provider OAuth callback accessible without catalog permission", async () => {
  useAuthStore.setState({
    user: {
      ...catalogUser,
      role: "user",
      is_operator: false,
      capabilities: undefined,
    },
  });
  const guard = providersRedirectRoute.options.beforeLoad!;
  await expect(
    Promise.resolve().then(() =>
      guard({ location: { pathname: "/providers/callback" } } as never),
    ),
  ).resolves.toBeUndefined();
});

describe("Tools management direct URL guard", () => {
  const guard = adminToolsRoute.options.beforeLoad!;
  it.each([
    [{ ...catalogUser, capabilities: { manage_tools: true } }, true],
    [
      {
        ...catalogUser,
        capabilities: { manage_tools: true, manage_catalog: true },
      },
      true,
    ],
    [
      {
        ...catalogUser,
        role: "admin",
        is_admin: true,
        capabilities: undefined,
      },
      true,
    ],
    [{ ...catalogUser, capabilities: { manage_catalog: true } }, false],
    [{ ...catalogUser, capabilities: undefined }, false],
    [null, false],
  ])("checks the live backend capability", async (user, allowed) => {
    useAuthStore.setState({ user: user as User | null });
    const result = Promise.resolve().then(() => guard({} as never));
    if (allowed) await expect(result).resolves.toBeUndefined();
    else
      await expect(result).rejects.toMatchObject({ options: { to: "/tools" } });
  });
});
