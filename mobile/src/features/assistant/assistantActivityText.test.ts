import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { describeToolActivity } from "./assistantActivityText";

// Web PR #1870 cases, run with the mobile package's existing Node test runner.
describe("describeToolActivity Web parity", () => {
  it("phrases a service operation for each status", () => {
    assert.equal(
      describeToolActivity("api-github-4__get_issue", "running"),
      "Reading issue on GitHub",
    );
    assert.equal(
      describeToolActivity("api-github-4__get_file_contents", "done"),
      "Read file contents on GitHub",
    );
    assert.equal(
      describeToolActivity("api-github-4__create_issue_comment", "error"),
      "Couldn't create issue comment on GitHub",
    );
  });

  it("finds a verb after a resource prefix and title-cases unbranded services", () => {
    assert.equal(
      describeToolActivity(
        "api-google-workspace-2__calendar_list_events",
        "done",
      ),
      "Listed calendar events on Google Workspace",
    );
    assert.equal(
      describeToolActivity("chrono-sandbox__execute_handler", "running"),
      "Running handler on Chrono Sandbox",
    );
  });

  it("uses the Web fallback for operations with no known verb", () => {
    assert.equal(
      describeToolActivity("llm-openai__chat_completions", "done"),
      "Used chat completions on OpenAI",
    );
  });

  it("describes NyxID tools without a service suffix", () => {
    assert.equal(
      describeToolActivity("nyx__search_tools", "running"),
      "Searching for tools",
    );
    assert.equal(
      describeToolActivity("nyx__search_tools", "error"),
      "Couldn't search for tools",
    );
    assert.equal(describeToolActivity("nyxid__remember", "done"), "Remembered");
    assert.equal(
      describeToolActivity("nyxid__grant_subagent", "running"),
      "Granting subagent",
    );
    assert.equal(
      describeToolActivity("nyx__discover_services", "done"),
      "Browsed the service catalog",
    );
    assert.equal(
      describeToolActivity("nyx__list_connected_services", "done"),
      "Checked connected services",
    );
  });

  it("keeps unknown identifiers without treating inherited object properties as phrases", () => {
    assert.equal(describeToolActivity("catalog.read", "done"), "catalog.read");
    assert.equal(describeToolActivity("tool", "running"), "tool");
    assert.equal(describeToolActivity("constructor", "done"), "constructor");
    assert.equal(
      describeToolActivity("nyxid__constructor", "done"),
      "Used constructor",
    );
    assert.equal(
      describeToolActivity("constructor__get_issue", "done"),
      "Read issue on Constructor",
    );
  });
});
