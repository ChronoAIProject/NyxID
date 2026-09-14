import { describe, expect, it } from "vitest";
import { loginRequestReturnTo, parseLoginRequestHints } from "./login-request";

describe("device approval link hints", () => {
  it("preserves bounded list hints and normalized public codes through identity login", () => {
    const query = new URLSearchParams({
      user_code: "ioLu-abcd",
      login_type: "agent",
      key_source: "new",
      key_name: "CLI account",
      expiry_days: "30",
      platform: "codex",
      services: "github",
      service_permissions: "github::repo:read",
    });
    query.append("permissions", "read,proxy");
    query.append("permissions", "email,read");
    const hints = parseLoginRequestHints(`?${query}`);
    expect(hints.errors).toEqual([]);
    expect(hints.permissions).toEqual(["read", "proxy", "email"]);
    expect(hints.user_code).toBe("101VABCD");
    const returned = loginRequestReturnTo("device", hints, hints.user_code!);
    expect(
      parseLoginRequestHints(returned.slice(returned.indexOf("?"))),
    ).toMatchObject({ ...hints, query: expect.any(String) });
  });
  it.each([
    "constructor=x",
    "__proto__=x",
    "toString=x",
    "%63onstructor=x",
    "supports_grant_choice=true",
    "login_type=agent&login_type=agent",
    "user_code=ABCDEFGH&user_code=12345678",
    "key_name=hello",
    "login_type=admin",
    "platform=unknown",
    "expiry_days=-1",
    "permissions=admin,",
    "permissions=unknown",
    "services=../github",
    "service_permissions=github::",
    "key_name=%FF",
    "key_name=%",
    "key_name=a%00b",
    "preview_keys=true",
    "redirect_uri=https://evil.example",
  ])("fails visibly on malformed or unsupported input %s", (query) => {
    const hints = parseLoginRequestHints(query);
    expect(hints.errors.length).toBeGreaterThan(0);
    expect(hints.query).toBe("");
  });
  it("bounds raw input and each decoded list", () => {
    for (const query of [
      "x".repeat(49153),
      `permissions=${"a".repeat(1025)}`,
      `services=${"a".repeat(4097)}`,
      `service_permissions=${"a".repeat(8193)}`,
    ])
      expect(parseLoginRequestHints(query).errors.length).toBeGreaterThan(0);
  });
  it("keeps agent-only requests restricted and old marker links readable", () => {
    expect(
      parseLoginRequestHints("user_code=2-ABCD-EFGH", "agent-key"),
    ).toMatchObject({
      errors: [],
      user_code: "2ABCDEFGH",
      login_type: "agent",
    });
    expect(
      parseLoginRequestHints("login_type=full", "agent-key").errors.length,
    ).toBeGreaterThan(0);
  });
});
