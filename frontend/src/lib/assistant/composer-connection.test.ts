import { afterEach, describe, expect, it } from "vitest";
import { appI18n } from "@/i18n/instance";
import en from "@/i18n/locales/app/shared/en.json";
import zhCN from "@/i18n/locales/app/shared/zh-CN.json";
import zhTW from "@/i18n/locales/app/shared/zh-TW.json";
import type { KeyInfo } from "@/types/keys";
import { connectionAvailable } from "./connection-context";
import {
  composerConnectionUnavailableReason,
  formatComposerConnectionReason,
} from "./composer-connection";

const connection = {
  id: "connection",
  is_active: true,
  status: "active",
  credential_type: "api_key",
  api_key_id: "credential",
  credential_source: { type: "personal" },
} as KeyInfo;
const now = Date.parse("2026-11-16");

describe("composer execution availability", () => {
  it.each<[Partial<KeyInfo>, string]>([
    [{ is_active: false }, "Disabled"],
    [
      { credential_binding: "platform", platform_key_available: false },
      "Platform unavailable",
    ],
    [
      { auto_connected: true, platform_key_available: false },
      "Platform unavailable",
    ],
    [{ credential_missing: true }, "Credential missing"],
    [{ status: "revoked" }, "Reconnect needed"],
    [{ status: "failed" }, "Reconnect needed"],
    [{ status: "refresh_failed" }, "Reconnect needed"],
    [{ status: "pending_auth" }, "Finish connecting"],
    [{ requires_connection: true, connected: false }, "Finish connecting"],
    [{ connection_status: "expired" }, "Reconnect needed"],
    [{ status: "expired" }, "Reconnect needed"],
    [{ expires_at: "2026-01-01" }, "Credential expired"],
    [
      {
        credential_source: {
          type: "org",
          org_id: "org",
          org_name: "Team",
          allowed: false,
          role: "viewer",
        },
      },
      "No access",
    ],
    [
      {
        node_id: "node",
        node_status: "offline",
        execution_unavailable_reason: "node home offline",
      },
      "node home offline",
    ],
  ])("blocks explicit unavailable facts: %j", (overrides, reason) => {
    expect(
      composerConnectionUnavailableReason({ ...connection, ...overrides }, now),
    ).toEqual(
      reason.startsWith("node ")
        ? { kind: "node", name: "home", status: "offline" }
        : { kind: "classifier", reason },
    );
  });

  it.each<Partial<KeyInfo>>([
    {},
    { credential_source: undefined, api_key_id: null },
    { credential_binding: "platform", platform_key_available: true },
    {
      credential_binding: "user",
      auto_connected: true,
      platform_key_available: false,
    },
    {
      credential_type: "oauth2",
      expires_at: "2026-01-01",
      connection_status: "active",
    },
    {
      node_id: "node",
      node_status: "offline",
      execution_unavailable_reason: null,
    },
    { node_id: "node", node_status: "online" },
  ])(
    "keeps normal, unverified, refreshable and fallback connections selectable: %j",
    (overrides) => {
      expect(
        composerConnectionUnavailableReason(
          { ...connection, ...overrides },
          now,
        ),
      ).toBeNull();
    },
  );

  it("does not narrow the durable grant predicate", () => {
    const offline = {
      ...connection,
      node_id: "node",
      node_status: "offline",
      execution_unavailable_reason: "node home offline",
    };
    expect(composerConnectionUnavailableReason(offline, now)).not.toBeNull();
    expect(connectionAvailable(offline)).toBe(true);
  });
});

it.each<Partial<KeyInfo>>([
  { is_active: false },
  { credential_missing: true },
  { credential_binding: "platform", platform_key_available: false },
  {
    credential_source: {
      type: "org",
      org_id: "org",
      org_name: "Team",
      allowed: false,
      role: "viewer",
    },
  },
])("keeps hard classifier facts ahead of backend node reasons: %j", (patch) => {
  const expected = composerConnectionUnavailableReason(
    { ...connection, ...patch },
    now,
  );
  expect(
    composerConnectionUnavailableReason(
      {
        ...connection,
        ...patch,
        execution_unavailable_reason: "node home offline",
      },
      now,
    ),
  ).toEqual(expected);
});

it("preserves node fallback classification even with expired server credentials", () => {
  expect(
    composerConnectionUnavailableReason(
      {
        ...connection,
        node_id: "node",
        node_status: "offline",
        status: "expired",
        expires_at: "2020-01-01",
      },
      now,
    ),
  ).toBeNull();
});

afterEach(async () => {
  await appI18n.changeLanguage("en");
});

it.each(["Provider says <b>wait</b> {{x}}", "  Provider says wait  "])(
  "keeps an unknown nonempty backend reason verbatim: %j",
  (text) => {
    const reason = composerConnectionUnavailableReason(
      { ...connection, execution_unavailable_reason: text },
      now,
    )!;
    expect(reason).toEqual({ kind: "backend", text });
    expect(formatComposerConnectionReason(reason)).toBe(text);
  },
);

it.each([
  ["en", en, "Connection unavailable"],
  ["zh-CN", zhCN, "连接不可用"],
  ["zh-TW", zhTW, "連線不可用"],
] as const)("keeps blank backend reasons blocking with visible localized copy in %s", async (language, catalog, expected) => {
  appI18n.addResourceBundle(language, "app", catalog, true, true);
  await appI18n.changeLanguage(language);
  for (const text of ["   ", " \t\n ", "\u00a0"]) {
    const reason = composerConnectionUnavailableReason(
      { ...connection, execution_unavailable_reason: text },
      now,
    )!;
    expect(reason).toEqual({ kind: "backend", text });
    expect(formatComposerConnectionReason(reason)).toBe(expected);
  }
});
