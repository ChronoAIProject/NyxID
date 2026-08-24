import { describe, expect, it } from "vitest";
import { sanitizeAssistantMessageContent } from "./chat-content";

describe("chat content", () => {
  it("strips complete and dangling tool call blocks", () => {
    expect(
      sanitizeAssistantMessageContent(
        "Before\n<function_calls><invoke>secret</invoke></function_calls>\nAfter",
      ),
    ).toBe("Before\n\nAfter");
    expect(
      sanitizeAssistantMessageContent(
        "Before\n<|DSML|function_calls><invoke>secret</invoke></|DSML|function_calls>\nAfter",
      ),
    ).toBe("Before\n\nAfter");
    expect(
      sanitizeAssistantMessageContent(
        "Visible answer\n<function_calls><invoke>unfinished",
      ),
    ).toBe("Visible answer");
    expect(
      sanitizeAssistantMessageContent(
        "Visible answer\n<｜DSML｜function_calls><invoke>unfinished",
      ),
    ).toBe("Visible answer");
  });

  it("collapses blank runs without trimming leading message content", () => {
    expect(sanitizeAssistantMessageContent("  indented\n\n\n\nnext  \n")).toBe(
      "  indented\n\nnext",
    );
  });
});
