import { describe, expect, it } from "vitest";
import { describeToolActivity } from "./tool-activity-text";

describe("describeToolActivity", () => {
  it("phrases a service operation for each status", () => {
    expect(describeToolActivity("api-github-4__get_issue", "running")).toBe("Reading issue on GitHub");
    expect(describeToolActivity("api-github-4__get_file_contents", "done")).toBe("Read file contents on GitHub");
    expect(describeToolActivity("api-github-4__create_issue_comment", "error")).toBe("Couldn't create issue comment on GitHub");
  });

  it("finds the verb after a resource prefix and title-cases unbranded services", () => {
    expect(describeToolActivity("api-google-workspace-2__calendar_list_events", "done")).toBe(
      "Listed calendar events on Google Workspace",
    );
    expect(describeToolActivity("chrono-sandbox__execute_handler", "running")).toBe("Running handler on Chrono Sandbox");
  });

  it("falls back to 'use' when the operation has no known verb", () => {
    expect(describeToolActivity("llm-openai__chat_completions", "done")).toBe("Used chat completions on OpenAI");
  });

  it("describes NyxID's own tools without a service suffix", () => {
    expect(describeToolActivity("nyx__search_tools", "running")).toBe("Searching for tools");
    expect(describeToolActivity("nyx__search_tools", "error")).toBe("Couldn't search for tools");
    expect(describeToolActivity("nyxid__remember", "done")).toBe("Remembered");
    expect(describeToolActivity("nyxid__grant_subagent", "running")).toBe("Granting subagent");
  });

  it("returns unrecognized identifiers unchanged", () => {
    expect(describeToolActivity("catalog.read", "done")).toBe("catalog.read");
    expect(describeToolActivity("tool", "running")).toBe("tool");
  });
});
