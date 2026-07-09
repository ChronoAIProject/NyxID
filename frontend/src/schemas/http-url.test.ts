import { describe, expect, it } from "vitest";
import { isValidHttpUrl } from "./http-url";

describe("isValidHttpUrl", () => {
  it("accepts real-world http(s) URLs with a TLD", () => {
    for (const url of [
      "https://example.com",
      "https://www.example.com/path?q=1#frag",
      "http://api.sub.domain.io:8080/v1",
      "https://example.com.",
      "https://ha.local",
    ]) {
      expect(isValidHttpUrl(url), url).toBe(true);
    }
  });

  it("accepts localhost and IP literals (self-hosted gateways)", () => {
    for (const url of [
      "http://localhost:18789",
      "http://127.0.0.1:8080",
      "https://192.168.1.20:8123/api",
      "http://[::1]:3001",
    ]) {
      expect(isValidHttpUrl(url), url).toBe(true);
    }
  });

  it("rejects TLD-less and partial hosts", () => {
    for (const url of [
      "https://www.",
      "https://www",
      "http://foo",
      "https://foo.",
      "https://foo-.com",
    ]) {
      expect(isValidHttpUrl(url), url).toBe(false);
    }
  });

  it("rejects non-http schemes, scheme-less input, and garbage", () => {
    for (const url of [
      "ftp://example.com",
      "wss://stream.example.com/socket",
      "javascript:alert(1)",
      "mailto:a@b.com",
      "www.example.com",
      "example.com",
      "not a url",
      "",
    ]) {
      expect(isValidHttpUrl(url), url).toBe(false);
    }
  });
});
