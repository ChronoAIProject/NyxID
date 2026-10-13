import { describe, expect, it } from "vitest";
import { describeToolActivity, toolServiceName } from "./tool-activity-labels";

describe("describeToolActivity", () => {
  it.each([
    ["nyx__search_tools", "Search available tools", "Searching available tools"],
    ["nyxid__remember", "Save a memory note", "Saving a memory note"],
    ["nyxid__spawn_subagent", "Start subagent", "Starting subagent"],
    ["api-github-4__get_issue", "Read issue on GitHub", "Reading issue on GitHub"],
    ["api-github-4__get_file_contents", "Read file contents on GitHub", "Reading file contents on GitHub"],
    ["api-google-workspace-2__calendar_list_events", "List calendar events on Google Workspace", "Listing calendar events on Google Workspace"],
    ["api-lark__im_message_create", "Create im message on Lark", "Creating im message on Lark"],
    ["llm-openai__chat_completions", "Run chat completions on OpenAI", "Running chat completions on OpenAI"],
    ["tool", "Run tool", "Running tool"],
  ])("%s", (name, action, progress) => {
    expect(describeToolActivity(name)).toEqual({ action, progress });
  });

  it("bounds long operation names", () => {
    const { action } = describeToolActivity(`api-x__get_${"a".repeat(200)}`);
    expect(action.length).toBeLessThan(90);
    expect(action).toContain("…");
  });
});

describe("toolServiceName", () => {
  it.each([
    ["api-github-4", "GitHub"],
    ["api-twitter", "X"],
    ["chrono-sandbox", "Chrono Sandbox"],
  ])("%s -> %s", (slug, name) => {
    expect(toolServiceName(slug)).toBe(name);
  });
});
