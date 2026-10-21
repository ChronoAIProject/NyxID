import { describe, expect, it } from "vitest";
import { runtimeConfigSchema } from "./runtime-config";

const config = {
  api_base_url: "https://api.example.com",
  release_integrity: { enabled: true, manifest_url: "https://releases.example.com/v1/releases.json", verification_ttl_secs: 300 },
};

describe("runtime transport URLs", () => {
  it("accepts a split HTTP(S) API origin and a manifest path", () => {
    expect(runtimeConfigSchema.parse(config)).toEqual(config);
    expect(runtimeConfigSchema.parse({ ...config, api_base_url: "http://[::1]:3001/" }).api_base_url).toBe("http://[::1]:3001");
    expect(runtimeConfigSchema.parse({ ...config, api_base_url: "https://API.example:443/" }).api_base_url).toBe("https://api.example");
  });
  it.each(["https:api.example", "https:/api.example", "https:///api.example", "https://api.example?", "https://api.example#", "javascript:alert(1)", "file:///tmp/secret", "https://user:pass@api.example.com", "https://api.example.com/base", "https://api.example.com?x=1", "https://api.example.com#x", "https://api.example.com\\evil"])("rejects unsafe API transport %s", (api_base_url) => {
    expect(runtimeConfigSchema.safeParse({ ...config, api_base_url }).success).toBe(false);
  });
  it("rejects credential-bearing and executable manifest URLs", () => {
    for (const manifest_url of ["https:example.com/releases.json", "https://example.com/releases.json#", "javascript:alert(1)", "https://user:pass@example.com/releases.json", "https://example.com/releases.json#secret"]) {
      expect(runtimeConfigSchema.safeParse({ ...config, release_integrity: { ...config.release_integrity, manifest_url } }).success).toBe(false);
    }
  });
});
