import { describe, expect, it } from "vitest";
import { normalizeScreenKey } from "./screen-context";

describe("normalizeScreenKey", () => {
  it.each([
    ["/dashboard", "/dashboard"],
    ["/keys", "/keys"],
    ["/keys/2f47d5a1-27e0-4f43-9941-93c322c93e9f", "/keys"],
    ["/keys/api-key/key-1", "/keys"],
    ["/api-keys", "/keys"],
    ["/services/service-1/edit", "/services"],
    ["/nodes/node-1", "/nodes"],
    ["/orgs/org-1/developer-apps/app-1", "/orgs"],
    ["/channel-bots/bot-1/conversations/conversation-1", "/channel-bots"],
    ["/developer/apps/client-1", "/developer/apps"],
    ["/developer/tools/tool-1", "/developer/tools"],
    ["/admin", "/admin"],
    ["/admin/users/user-1", "/admin/users"],
    ["/admin/audit-log", "/admin/audit-log"],
    ["/approvals/history/request-1", "/approvals/history"],
    ["/settings/consents", "/settings/consents"],
    ["/settings/authorizations", "/settings/consents"],
    ["/devices/onboard", "/devices/onboard"],
    ["/integration-guide/", "/integration-guide"],
    ["/future-dashboard/detail", "/future-dashboard"],
  ])("normalizes %s to %s", (pathname, expected) => {
    expect(normalizeScreenKey(pathname)).toBe(expected);
  });

  it.each([
    "/",
    "/assistant",
    "/assistant/plugins",
    "/login",
    "/register",
    "/docs/web/getting-started",
    "/blog/release",
    "/privacy",
    "/terms",
    "/oauth-consent",
    "/ssh/service-1/terminal",
  ])("does not record the non-dashboard surface %s", (pathname) => {
    expect(normalizeScreenKey(pathname)).toBeNull();
  });
});
