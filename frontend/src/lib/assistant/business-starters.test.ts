import { describe, expect, it } from "vitest";
import { composeBusinessStarter } from "./business-starters";
const cases = [
  {
    id: "queue-manager",
    title: "Queue manager",
    description: "Queue",
    example_prompts: ["Set up my queue", "Second prompt"],
  },
  {
    id: "business-faq",
    title: "Business FAQ",
    description: "FAQ",
    example_prompts: ["Set up my FAQ"],
  },
  {
    id: "empty",
    title: "Empty",
    description: "No prompt",
    example_prompts: [],
  },
];
describe("business starter text", () => {
  it("uses only the first published prompt", () =>
    expect(composeBusinessStarter("queue-manager", cases)).toBe(
      "Set up my queue",
    ));
  it("uses requested order and titles for multiple matches", () =>
    expect(
      composeBusinessStarter("business-faq,queue-manager,queue-manager", cases),
    ).toBe(
      "Set up these use cases for my business:\n- Business FAQ: Set up my FAQ\n- Queue manager: Set up my queue",
    ));
  it("ignores unknown ids and missing prompts", () => {
    expect(composeBusinessStarter("unknown,empty", cases)).toBe("");
    expect(composeBusinessStarter("unknown,queue-manager,empty", cases)).toBe(
      "Set up my queue",
    );
  });
});
