import { describe, expect, it } from "vitest";
import {
  assistantInputRequestSchema,
  buildInputResolveBody,
  inputAnswerSchema,
  inputResolveBodySchema,
} from "./assistant-input";

describe("assistant input schemas", () => {
  it("builds each closed answer variant with a positive observed version", () => {
    expect(
      buildInputResolveBody(
        "nyxid-chat-1",
        "client-1",
        "input-1",
        { freeText: "  Singapore  " },
        23,
      ),
    ).toEqual({
      type: "input.resolve",
      conversationId: "nyxid-chat-1",
      clientRequestId: "client-1",
      requestId: "input-1",
      answer: { freeText: "Singapore" },
      expectedStateVersion: 23,
    });
    expect(
      buildInputResolveBody(
        "nyxid-chat-1",
        "client-2",
        "input-2",
        { selectedOptionIds: ["option-a", "option-b"] },
        24,
      ).answer,
    ).toEqual({ selectedOptionIds: ["option-a", "option-b"] });
  });

  it("rejects mixed, empty, duplicate and over-broad answers", () => {
    for (const answer of [
      {},
      { freeText: "" },
      { freeText: "answer", selectedOptionIds: ["option-a"] },
      { selectedOptionIds: [] },
      { selectedOptionIds: ["option-a", "option-a"] },
      {
        selectedOptionIds: [
          "option-a",
          "option-b",
          "option-c",
          "option-d",
          "option-e",
          "option-f",
          "option-g",
        ],
      },
    ]) {
      expect(inputAnswerSchema.safeParse(answer).success).toBe(false);
    }
    expect(
      inputResolveBodySchema.safeParse({
        type: "input.resolve",
        conversationId: "nyxid-chat-1",
        clientRequestId: "client-1",
        requestId: "input-1",
        answer: { freeText: "answer" },
        expectedStateVersion: 0,
      }).success,
    ).toBe(false);
  });

  it("accepts only an actionable safe input request", () => {
    expect(
      assistantInputRequestSchema.safeParse({
        requestId: "input-1",
        prompt: "Choose a region",
        options: [
          {
            optionId: "option-sg",
            label: "Singapore",
            additiveHint: "future-field",
          },
          { optionId: "option-fra", label: "Frankfurt" },
        ],
        allowFreeText: false,
        multiSelect: true,
        turnId: "turn-1",
      }).success,
    ).toBe(true);
    expect(
      assistantInputRequestSchema.safeParse({
        requestId: "input-1",
        prompt: "Choose a region",
        options: [],
        allowFreeText: false,
        multiSelect: false,
      }).success,
    ).toBe(false);
  });

  it("accepts the plan gate's single proceed option with free-text revision", () => {
    expect(
      assistantInputRequestSchema.safeParse({
        requestId: "plan-gate-1",
        prompt: "Proceed with this plan or describe a revision",
        options: [{ optionId: "proceed", label: "Proceed" }],
        allowFreeText: true,
        multiSelect: false,
      }).success,
    ).toBe(true);

    expect(
      assistantInputRequestSchema.safeParse({
        requestId: "invalid-single-choice-1",
        prompt: "Choose an option",
        options: [{ optionId: "only", label: "Only option" }],
        allowFreeText: false,
        multiSelect: false,
      }).success,
    ).toBe(false);
  });
});
