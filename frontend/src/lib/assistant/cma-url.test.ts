import { expect, it } from "vitest";
import { safeCmaUrl } from "./cma-url";

it.each([
  "https://cma.example:8443/agents/agt_x",
  "http://localhost:8080/bots/agt_x",
  "https://example.com/%E4%BD%A0",
  "HTTPS://CMA.example/bots/agt_x",
])("preserves a valid supplied browser destination: %s", (value) => {
  expect(safeCmaUrl(value)).toBe(value);
});
it.each([
  "https://cma.example/agent?token=secret",
  "https://cma.example/agent#fragment",
  "https://cma.example/agent?",
  "https://cma.example/agent#",
  "//example.com",
  "/agents/agt_x",
  "javascript:alert(1)",
  "file:///tmp/a",
  "https://user:password@example.com",
  "https://user@example.com",
  "https://@example.com",
  "https://example.com\\@evil.test",
  "https://example.com/\nagent",
  "https://example.com/\u0085agent",
  "https://example.com\t",
  " https://example.com",
])("rejects an unsafe callback: %s", (value) =>
  expect(safeCmaUrl(value)).toBeUndefined(),
);
