import { describe, expect, it } from "vitest";
import { redactAssistantDisplayText } from "./chat-display-safety";

describe("assistant display redaction", () => {
  it.each([
    ['{"api_key":"sk-live-12345","status":"ok"}', "sk-live-12345"],
    ["Authorization: Bearer abc.def.ghi", "abc.def.ghi"],
    ["key nyxid_ag_supersecret1234", "nyxid_ag_supersecret1234"],
    ["Authorization: Basic dXNlcjpwYXNzd29yZA==", "dXNlcjpwYXNzd29yZA=="],
    ['{"secretAccessKey":"wJalrXUtnFEMI/K7MDENG"}', "wJalrXUtnFEMI/K7MDENG"],
    ['{"accessKeyId":"AKIAIOSFODNN7EXAMPLE"}', "AKIAIOSFODNN7EXAMPLE"],
    ['{"password":"correct horse battery staple"}', "correct horse battery staple"],
    ["OpenAI rejected sk-proj-abc123def456ghi", "sk-proj-abc123def456ghi"],
    ["push failed for ghp_abcdefghij1234567890KLMN", "ghp_abcdefghij1234567890KLMN"],
  ])("redacts secret-bearing display text %#", (input, secret) => {
    expect(redactAssistantDisplayText(input)).not.toContain(secret);
  });
});
