import assert from "node:assert/strict";
import { test } from "node:test";
import {
  parsePublicAuthConfig,
  requireSocialProvider,
} from "./publicAuthConfig";

for (const emailEnabled of [false, true]) {
  for (let subset = 0; subset < 8; subset++) {
    const providers = (["google", "github", "apple"] as const)
      .filter((_, i) => (subset & (1 << i)) !== 0)
      .reverse();
    test(`public config parses provider subset ${subset} and email ${emailEnabled ? "on" : "off"}`, () => {
      const config = parsePublicAuthConfig({
        email_auth_enabled: emailEnabled,
        social_providers: providers,
      });
      assert.deepEqual(config, { emailAuthEnabled: emailEnabled, socialProviders: providers });
      for (const provider of ["google", "github", "apple"] as const) {
        if (config.socialProviders.includes(provider)) {
          assert.doesNotThrow(() => requireSocialProvider(config, provider));
        } else {
          assert.throws(() => requireSocialProvider(config, provider), /not enabled on this server/);
        }
      }
    });
  }
}

test("a local email-only deployment does not enable social login", () => {
  const config = parsePublicAuthConfig({
    email_auth_enabled: true,
    social_providers: [],
  });
  assert.deepEqual(config, { emailAuthEnabled: true, socialProviders: [] });
  assert.throws(
    () => requireSocialProvider(config, "google"),
    /not enabled on this server/,
  );
});

test("a social-only deployment preserves its enabled login methods", () => {
  const config = parsePublicAuthConfig({
    email_auth_enabled: false,
    social_providers: ["google", "github", "apple"],
    version: "test",
  });
  assert.deepEqual(config, {
    emailAuthEnabled: false,
    socialProviders: ["google", "github", "apple"],
  });
  for (const provider of config.socialProviders) {
    assert.doesNotThrow(() => requireSocialProvider(config, provider));
  }
});

test("unknown providers are ignored and supported providers are deduplicated", () => {
  assert.deepEqual(
    parsePublicAuthConfig({
      email_auth_enabled: false,
      social_providers: [
        "future-provider",
        "github",
        "Google",
        "github",
        "apple",
        "google",
      ],
    }),
    { emailAuthEnabled: false, socialProviders: ["github", "apple", "google"] },
  );
});

test("missing or malformed configuration fails closed with safe feedback", () => {
  const marker = "private-response-marker";
  for (const payload of [
    null,
    {},
    { social_providers: ["google"] },
    {
      email_auth_enabled: "true",
      social_providers: ["google"],
      message: marker,
    },
    { email_auth_enabled: null, social_providers: ["google"] },
    { email_auth_enabled: true },
    { email_auth_enabled: true, social_providers: "google" },
    { email_auth_enabled: true, social_providers: ["google", null] },
  ]) {
    assert.throws(
      () => parsePublicAuthConfig(payload),
      (error) => {
        assert.ok(error instanceof Error);
        assert.equal(
          error.message,
          "Unable to load sign-in options. Please try again.",
        );
        assert.equal(error.message.includes(marker), false);
        return true;
      },
    );
  }
});

test("social sign-in cannot start before configuration or for a disabled provider", () => {
  for (const config of [null, undefined]) {
    assert.throws(() => requireSocialProvider(config, "github"), /not ready/);
  }
  const config = parsePublicAuthConfig({
    email_auth_enabled: true,
    social_providers: ["github"],
  });
  assert.doesNotThrow(() => requireSocialProvider(config, "github"));
  assert.throws(
    () => requireSocialProvider(config, "apple"),
    /Apple sign-in is not enabled/,
  );
});
