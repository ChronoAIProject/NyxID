import { describe, expect, it } from "vitest";

import { buildCliAuthReturnPath, cliCallbackUrl, validateCliAuthUrl } from "./cli-auth.helpers";

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

describe("CLI callback port", () => {
  it.each(["1e2", "+80", " 80 ", '"80"', "80\n"])("validates %j before router parsing", (port) => {
    const url = new URL("http://localhost/cli-auth");
    url.searchParams.set("port", port);
    expect(validateCliAuthUrl(url).searchParams.has("port")).toBe(false);
  });

  it("requires one port and accepts both boundaries", () => {
    const duplicate = new URL("http://localhost/cli-auth?port=80&port=81");
    expect(validateCliAuthUrl(duplicate).searchParams.has("port")).toBe(false);
    for (const port of ["1", "65535"]) {
      expect(cliCallbackUrl(port)?.href).toBe(`http://127.0.0.1:${port}/callback`);
    }
  });
});
