import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { URL as NodeURL } from "node:url";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { ApiError } from "./ApiError";
import { createSessionRefresh } from "../auth/sessionRefresh";
import { executeAuthenticatedRequest } from "./authenticatedTransport";
import { guardedJsonTransport } from "./guardedTransport";
type ApiUrlModule = typeof import("./baseUrl");
type HttpModule = typeof import("./http");
type MobileApiModule = typeof import("./mobileApi");

export function loadTestModule<T>(filename: string, context: Record<string, unknown>): T {
  const source = readFileSync(new NodeURL(filename, import.meta.url), "utf8");
  const js = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exported = {};
  runInNewContext(js, { exports: exported, ...context });
  return exported as T;
}

export function loadApiConsumers(api: ApiUrlModule, fetch: typeof globalThis.fetch, loadModule = loadTestModule) {
  let session = { accessToken: "access-before-refresh", refreshToken: "refresh-test", sessionGeneration: 1 };
  const dependencies: Record<string, unknown> = {
    "./connectionState": { assistantConnection: { observe: (send: () => Promise<unknown>) => send() } },
    "../auth/sessionRefresh": { createSessionRefresh },
    "expo/fetch": { fetch },
    "./authenticatedTransport": { executeAuthenticatedRequest },
    "./approvalAdapter": {},
    "../auth/authEpoch": { getAuthEpoch: () => 0 },
    "./readBoundedBytes": {},
    "./guardedTransport": { guardedJsonTransport },
    "../analyticsConsentState": { analyticsDeviceHeaders: () => ({}) },
    "../auth/jwt": {},
    "../auth/mfaChallenge": {},
    "./ApiError": { ApiError },
    "./baseUrl": api,
    "expo-constants": { default: { expoConfig: { version: "test" } } },
    "../accountDeletionPolicy": { ACCOUNT_DELETION_POLICY_VERSION: "test" },
    "../auth/sessionStore": {
      loadStoredAuthSession: async () => session,
      authSessionGeneration: () => 1,
      commitRefreshedAuthSession: async (_expected: unknown, next: typeof session) => {
        session = { ...next, sessionGeneration: 1 };
        return true;
      },
      clearStoredAuthSession: async () => {},
    },
    "./authDeviceApi": {},
    "./idempotency": {},
  };
  const requireDependency = (name: string) => {
    assert.ok(Object.hasOwn(dependencies, name), `Unexpected API dependency: ${name}`);
    return dependencies[name];
  };
  const http = loadModule<HttpModule>("./http.ts", {
    require: requireDependency, fetch, __DEV__: false, setTimeout,
  });
  dependencies["./http"] = http;
  const { mobileApi } = loadModule<MobileApiModule>("./mobileApi.ts", { require: requireDependency });
  return { http, mobileApi };
}
