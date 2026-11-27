import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { DecisionReview } from "./decision-review";
import { DecisionCardInteractionBlocked } from "./decision-card-presentation";
import { DecisionStack } from "./decision-stack";
import { CardLanguageContext } from "./card-language";
import { appI18n } from "@/i18n";

afterEach(cleanup);

it("keeps the conversation's card language in the stack and its review portal", async () => {
  await appI18n.loadLanguages("zh-TW");
  render(<CardLanguageContext.Provider value="zh-TW">
    <DecisionStack items={[{ id: "draft", content: <DecisionReview title="User draft" summary="Summary" actions={<button>Allow reviewed draft</button>}>Full request</DecisionReview> }]} onEmptyFocus={vi.fn()} />
  </CardLanguageContext.Provider>);
  expect(document.querySelector("[data-decision-stack]")).toHaveAttribute("lang", "zh-TW");
  fireEvent.click(screen.getByRole("button", { name: "審閱完整請求" }));
  expect(screen.getByRole("dialog", { name: "User draft" })).toHaveAttribute("lang", "zh-TW");
});

it("offers Review before Allow and restores focus when the full review is closed", async () => {
  const allow = vi.fn();
  render(<DecisionReview title="Review draft" summary={<p>Publish privately</p>} actions={<button onClick={allow}>Allow reviewed draft</button>} onDeny={vi.fn()}>
    <p>Complete request contents</p>
  </DecisionReview>);
  expect(screen.queryByRole("button", { name: "Allow reviewed draft" })).not.toBeInTheDocument();
  expect(screen.queryByText("Complete request contents")).not.toBeInTheDocument();
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Review full request" }));
  const dialog = screen.getByRole("dialog", { name: "Review draft" });
  expect(within(dialog).getByText("Complete request contents")).toBeVisible();
  expect(within(dialog).getByRole("button", { name: "Allow reviewed draft" })).toBeVisible();
  expect(allow).not.toHaveBeenCalled();
  await user.keyboard("{Escape}");
  expect(screen.getByRole("button", { name: "Review full request" })).toHaveFocus();
});

it("applies the promotion guard to decision buttons rendered in the dialog portal", async () => {
  const allow = vi.fn();
  const card = <DecisionReview title="Review draft" summary="Summary" actions={<button onClick={allow}>Allow reviewed draft</button>}>Full request</DecisionReview>;
  const view = render(<DecisionCardInteractionBlocked value={false}>{card}</DecisionCardInteractionBlocked>);
  fireEvent.click(screen.getByRole("button", { name: "Review full request" }));
  view.rerender(<DecisionCardInteractionBlocked value>{card}</DecisionCardInteractionBlocked>);
  const button = screen.getByRole("button", { name: "Allow reviewed draft" });
  expect(button).toBeDisabled();
  await userEvent.click(button);
  expect(allow).not.toHaveBeenCalled();
});

it("closes a review if its request leaves the stack or another request is selected", async () => {
  const content = <DecisionReview title="Review draft" summary="Summary" actions={<button>Allow reviewed draft</button>}>Full request</DecisionReview>;
  const items = [{ id: "draft", content }, { id: "other", content: <button>Allow other</button> }];
  const view = render(<DecisionStack items={items} onEmptyFocus={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Review full request" }));
  expect(screen.getByRole("dialog")).toBeVisible();
  view.rerender(<DecisionStack items={items} selectedId="other" revealKey="other" onEmptyFocus={vi.fn()} />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Allow other" })).toBeVisible();
});
