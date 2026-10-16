import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import test from "node:test";
import ts from "typescript";
import {
  catalogEntrySchema,
  serviceConnectionSchema,
} from "../../features/services/serviceContracts";
import type { servicesApi } from "./servicesApi";

/** Execute the actual adapter, replacing only its native HTTP boundary. */
function loadServicesApi(requestJson: (path: string) => Promise<unknown>) {
  const filename = fileURLToPath(new URL("./servicesApi.ts", import.meta.url));
  const require = createRequire(filename);
  const output = ts.transpileModule(readFileSync(filename, "utf8"), {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText;
  const exports: Record<string, unknown> = {};
  runInNewContext(
    output,
    {
      exports,
      URLSearchParams,
      require: (name: string) => {
        if (name === "./http") return { requestJson };
        if (name === "../../features/services/serviceContracts")
          return { catalogEntrySchema, serviceConnectionSchema };
        return require(name);
      },
    },
    { filename },
  );
  return exports.servicesApi as typeof servicesApi;
}

test("OAuth repair preserves connection, organization and exact custom scopes on the wire", async () => {
  let requestedPath = "";
  const api = loadServicesApi(async (path) => {
    requestedPath = path;
    return { authorization_url: "https://provider.example/authorize" };
  });
  const response = await api.initiateOAuth(
    "provider/one",
    "connection/one",
    "org/team",
    ["repo", "custom:assistant-scope", "openid"],
  );
  const url = new URL(requestedPath, "https://nyxid.example");
  assert.equal(url.pathname, "/providers/provider%2Fone/connect/oauth");
  assert.equal(url.searchParams.get("key_id"), "connection/one");
  assert.equal(url.searchParams.get("target_org_id"), "org/team");
  assert.equal(
    url.searchParams.get("scope_override"),
    "repo,custom:assistant-scope,openid",
  );
  assert.equal(url.searchParams.has("scope"), false);
  assert.equal(
    response.authorization_url,
    "https://provider.example/authorize",
  );
});

test("OAuth repair sends an explicit empty override instead of silently adopting defaults", async () => {
  let requestedPath = "";
  const api = loadServicesApi(async (path) => {
    requestedPath = path;
    return { authorization_url: "https://provider.example/authorize" };
  });
  await api.initiateOAuth("provider", "connection", undefined, []);
  const url = new URL(requestedPath, "https://nyxid.example");
  assert.equal(url.searchParams.has("scope_override"), true);
  assert.equal(url.searchParams.get("scope_override"), "");
  assert.equal(url.searchParams.has("target_org_id"), false);
});

test("OAuth creation without an override keeps the existing provider-default contract", async () => {
  let requestedPath = "";
  const api = loadServicesApi(async (path) => {
    requestedPath = path;
    return { authorization_url: "https://provider.example/authorize" };
  });
  await api.initiateOAuth("provider", "new-connection");
  const url = new URL(requestedPath, "https://nyxid.example");
  assert.equal(url.searchParams.get("key_id"), "new-connection");
  assert.equal(url.searchParams.has("scope_override"), false);
});

test("provider-policy rejection reaches the caller without filtering or retrying scopes", async () => {
  let requests = 0;
  const policyError = new Error("The platform app cannot request this scope");
  const api = loadServicesApi(async () => {
    requests++;
    throw policyError;
  });
  await assert.rejects(
    api.initiateOAuth("provider", "connection", undefined, ["custom:scope"]),
    (error) => error === policyError,
  );
  assert.equal(requests, 1);
});
