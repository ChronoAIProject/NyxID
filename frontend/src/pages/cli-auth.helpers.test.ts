import { describe, expect, it } from "vitest";

import { buildCliAuthReturnPath } from "./cli-auth.helpers";

describe("buildCliAuthReturnPath", () => {
  it("preserves the CLI user agent across the login redirect", () => {
    expect(
      buildCliAuthReturnPath({
        port: "43123",
        state: "deadbeef",
        client_ua: "nyxid-cli/0.1.0",
      }),
    ).toBe("/cli-auth?port=43123&state=deadbeef&client_ua=nyxid-cli%2F0.1.0");
  });

  it("omits empty query parameters", () => {
    expect(buildCliAuthReturnPath({})).toBe("/cli-auth");
  });
});
