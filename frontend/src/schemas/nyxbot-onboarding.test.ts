import { beforeEach, describe, expect, it } from "vitest";
import type { KeyInfo } from "@/types/keys";
import {
  GOOGLE_WORKSPACE_SCOPES,
  hasWorkspaceAuthorization,
  nyxbotSearchSchema,
  readNyxbotProgress,
  saveNyxbotProgress,
} from "./nyxbot-onboarding";

const key = {
  id: "google-1",
  catalog_service_slug: "api-google",
  status: "active",
  is_active: true,
  granted_scopes: GOOGLE_WORKSPACE_SCOPES,
} as unknown as KeyInfo;

beforeEach(() => sessionStorage.clear());

describe("Nyxbot authorization boundaries", () => {
  it("requires both Drive file access and Calendar access, not identity sign-in", () => {
    expect(hasWorkspaceAuthorization(key)).toBe(true);
    expect(
      hasWorkspaceAuthorization({
        ...key,
        granted_scopes: ["openid", "email", "profile"],
      }),
    ).toBe(false);
    expect(
      hasWorkspaceAuthorization({
        ...key,
        granted_scopes: [GOOGLE_WORKSPACE_SCOPES[0]],
      }),
    ).toBe(false);
    expect(
      hasWorkspaceAuthorization({
        ...key,
        granted_scopes: [GOOGLE_WORKSPACE_SCOPES[1]],
      }),
    ).toBe(false);
  });
  it("accepts an existing broader Drive grant without requesting one", () => {
    expect(
      hasWorkspaceAuthorization({
        ...key,
        granted_scopes: [
          "https://www.googleapis.com/auth/drive",
          GOOGLE_WORKSPACE_SCOPES[1],
        ],
      }),
    ).toBe(true);
  });
  it.each([
    { status: "pending_auth" },
    { status: "failed" },
    { is_active: false },
    { connection_status: "expired" },
    { credential_missing: true },
    { connected: false },
    { catalog_service_slug: "another-service" },
    { credential_source: { type: "org", allowed: true } },
  ])("rejects inactive or non-personal connections: %j", (patch) => {
    expect(hasWorkspaceAuthorization({ ...key, ...patch } as KeyInfo)).toBe(
      false,
    );
  });
  it("does not accept completion or session claims from query parameters", () => {
    expect(
      nyxbotSearchSchema.parse({
        channel: "telegram",
        completed: "true",
        botId: "x",
        session: "x",
      }),
    ).toEqual({ channel: "telegram" });
    expect(
      nyxbotSearchSchema.parse({ channel: "instagram" }).channel,
    ).toBeUndefined();
  });
  it("only accepts account and source return hints, never a channel or completion shortcut", () => {
    expect(nyxbotSearchSchema.parse({ step: "source" }).step).toBe("source");
    expect(nyxbotSearchSchema.parse({ step: "account" }).step).toBe("account");
    expect(nyxbotSearchSchema.parse({ step: "channel" }).step).toBeUndefined();
    expect(nyxbotSearchSchema.parse({ step: "complete" }).step).toBeUndefined();
  });
  it("isolates progress by user and persists only non-secret resource references", () => {
    saveNyxbotProgress("a", {
      channel: "telegram",
      googleKeyId: "g",
      botId: "b",
      bot_token: "secret",
    } as never);
    expect(readNyxbotProgress("a")).toEqual({
      channel: "telegram",
      googleKeyId: "g",
      botId: "b",
    });
    expect(readNyxbotProgress("b").botId).toBeNull();
    expect(sessionStorage.getItem("nyxbot-onboarding:a")).not.toContain(
      "secret",
    );
    sessionStorage.setItem("nyxbot-onboarding:a", "invalid json");
    expect(readNyxbotProgress("a").googleKeyId).toBeNull();
  });
});
