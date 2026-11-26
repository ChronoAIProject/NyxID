import assert from "node:assert/strict";
import test from "node:test";
import { parsePublicAuthConfig } from "../../features/auth/publicAuthConfig";
import { localizedError } from "../../i18n/localizedError";
import { loadTestModule } from "./apiUrlTestHarness";

test("sign-in config caches its optional crash hint using the existing request", async () => {
  for (const hint of [undefined, true]) {
    const response = {
      email_auth_enabled: true,
      social_providers: [],
      ...(hint === undefined ? {} : { crash_diagnostics_offered: hint }),
    };
    let reads = 0;
    const hints: unknown[] = [];
    const api = loadTestModule<typeof import("./publicAuthApi")>("./publicAuthApi.ts", {
      require: (name: string) => {
        if (name === "../../features/auth/publicAuthConfig") return { parsePublicAuthConfig };
        if (name === "../../i18n/localizedError") return { localizedError };
        if (name === "../publicConfigHints") return {
          cachePublicConfigHints: (value: unknown) => hints.push(value),
        };
        if (name === "./http") return {
          requestJson: async () => { reads++; return response; },
        };
        throw new Error(`Unexpected dependency: ${name}`);
      },
      AbortController, setTimeout, clearTimeout,
    });
    const config = await api.getPublicAuthConfig();
    assert.equal(config.emailAuthEnabled, true);
    assert.equal(reads, 1);
    assert.deepEqual(hints, [response]);
  }
});
