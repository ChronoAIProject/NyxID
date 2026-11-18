import assert from "node:assert/strict";
import test from "node:test";
import { voiceSpeechText } from "./voiceSpeechText";

test("speech uses prose, link labels and inline code without markdown or raw URLs", () => {
  assert.equal(
    voiceSpeechText(
      "# **Hello** _crew_\n\nRead [the plan](https://example.com/private?q=secret), then `confirm`.",
    ),
    "Hello crew Read the plan, then confirm.",
  );
  assert.equal(
    voiceSpeechText("Visit https://example.com/private?q=secret."),
    "Visit link.",
  );
});
test("speech shortens fenced and indented code and strips internal tool markup", () => {
  assert.equal(
    voiceSpeechText("Before.\n\n```sh\nrm -rf /secret\n```\n\nAfter."),
    "Before. Code omitted. After.",
  );
  assert.equal(voiceSpeechText("    hidden code\n"), "Code omitted.");
  assert.equal(
    voiceSpeechText(
      "Hello <function_calls>private tool call</function_calls> crew",
    ),
    "Hello crew",
  );
});
test("speech reads tables by row and uses friendly mention labels", () => {
  assert.equal(
    voiceSpeechText(
      "| Name | Status |\n| --- | --- |\n| Alpha | Done |\n| Beta | Waiting |",
    ),
    "Name, Status Alpha, Done Beta, Waiting",
  );
  assert.equal(
    voiceSpeechText(
      "Ask @researcher, [@Designer](agent:abc), <@123|Alex> and <@456>.",
    ),
    "Ask researcher, Designer, Alex and someone.",
  );
});
