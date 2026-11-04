import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { LiveReplay } from "./live-replay";

// Reduced motion: the replay shows its finished scene at once.
vi.mock("@/hooks/use-media-query", () => ({ useMediaQuery: () => true }));
vi.mock("@/components/assistant/chat-message", () => ({
  ChatMessageBubble: ({ message }: { message: { content: string; continuation_of?: string } }) => (
    <p>{message.continuation_of ? "Continuing after your decision" : message.content}</p>
  ),
}));

afterEach(cleanup);

it("the chief-of-staff example follows the first picked topic", () => {
  render(<LiveReplay kind="chief-of-staff" topics={["calendar", "inbox"]} label="Example" />);
  expect(screen.getByText("Plan my day")).toBeTruthy();
  expect(screen.getByText(/Three meetings and two free hours/)).toBeTruthy();
  expect(screen.getByText("Example", { selector: "figcaption" })).toBeTruthy();
});

it("the control example ends with the allowed card and the finished action", () => {
  render(<LiveReplay kind="control" label="Example" />);
  expect(screen.getByText(/You allowed/)).toBeTruthy();
  expect(screen.getByText("Continuing after your decision")).toBeTruthy();
  expect(screen.getByText(/the agent key \*\*ci-bot\*\* is deleted/)).toBeTruthy();
});
