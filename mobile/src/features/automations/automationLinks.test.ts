import assert from "node:assert/strict";
import test from "node:test";
import { parseAutomationLink } from "./automationLinks";

const frontend = "https://nyx.example";
const setup = "4b47953b-92cc-482f-8443-e0bcc30ade21";

test("trusted setup links open read-only Automations and preserve only the agent filter", () => {
  for (const path of ["/assistant/automations", "/automations"]) {
    assert.deepEqual(
      parseAutomationLink(
        `${frontend}${path}?setup=${setup}&agent=researcher&instruction=private&token=secret&return_to=https://other.example#review`,
        frontend,
      ),
      { kind: "automation", params: { agentId: "researcher" } },
    );
  }
});

test("an ordinary automation link can open the list or an agent filter", () => {
  assert.deepEqual(parseAutomationLink(`${frontend}/automations`, frontend), {
    kind: "automation",
    params: {},
  });
  assert.deepEqual(
    parseAutomationLink(`${frontend}/automations?agent=agent%20one`, frontend),
    { kind: "automation", params: { agentId: "agent one" } },
  );
});

test("origin matching rejects external, lookalike, credential and protocol destinations", () => {
  for (const url of [
    `https://other.example/automations?setup=${setup}`,
    `https://nyx.example.other.example/automations?setup=${setup}`,
    `https://sub.nyx.example/automations?setup=${setup}`,
    `https://nyx.example:444/automations?setup=${setup}`,
    `http://nyx.example/automations?setup=${setup}`,
    `https://user@nyx.example/automations?setup=${setup}`,
    `https://user:password@nyx.example/automations?setup=${setup}`,
    `/automations?setup=${setup}`,
    `nyxid://automations?setup=${setup}`,
    "not a URL",
  ]) {
    assert.equal(parseAutomationLink(url, frontend), null, url);
  }
  for (const configured of [
    "",
    "invalid",
    "http://nyx.example",
    "https://user@nyx.example",
  ]) {
    assert.equal(
      parseAutomationLink(`${frontend}/automations?setup=${setup}`, configured),
      null,
    );
  }
});

test("only automation list paths are intercepted from chat", () => {
  for (const path of [
    "/automations/",
    "/assistant/automations/extra",
    "/automations/edit",
    "/automations-elsewhere",
    "/settings",
  ]) {
    assert.equal(parseAutomationLink(`${frontend}${path}`, frontend), null);
  }
});

test("invalid or ambiguous setup parameters become native errors, never an authoring route", () => {
  for (const query of [
    "setup=",
    "setup=not-a-uuid",
    "setup=------------------------------------",
    `setup=${setup}x`,
    `setup=${setup}&setup=${setup}`,
  ]) {
    const result = parseAutomationLink(
      `${frontend}/automations?${query}`,
      frontend,
    );
    assert.equal(result?.kind, "invalid", query);
    assert.ok(
      result?.kind === "invalid" && result.message.includes("new link"),
    );
  }
});

test("agent references are bounded and duplicates or control characters are rejected", () => {
  assert.deepEqual(
    parseAutomationLink(
      `${frontend}/automations?agent=${"a".repeat(64)}`,
      frontend,
    ),
    { kind: "automation", params: { agentId: "a".repeat(64) } },
  );
  for (const query of [
    "agent=",
    "agent=%20%20",
    "agent=one&agent=two",
    "agent=one%0Atwo",
    "agent=one%00two",
    `agent=${"a".repeat(65)}`,
  ]) {
    const result = parseAutomationLink(
      `${frontend}/automations?${query}`,
      frontend,
    );
    assert.equal(result?.kind, "invalid", query);
  }
});
