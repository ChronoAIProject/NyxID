import { useRef, useState } from "react";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { DecisionStack } from "./decision-stack";

afterEach(cleanup);
const items = ["a", "b", "c"].map((id) => ({ id, content: <div><label>{`Draft ${id}`}<input /></label><button>{`Allow ${id}`}</button></div> }));

it("renders nothing with no decisions and omits paging for a single request", () => {
  const view = render(<DecisionStack items={[]} onEmptyFocus={vi.fn()} />);
  expect(view.container).toBeEmptyDOMElement();
  view.rerender(<DecisionStack items={items.slice(0, 1)} onEmptyFocus={vi.fn()} />);
  expect(screen.getByRole("region", { name: "Needs your decision" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Next decision" })).not.toBeInTheDocument();
});

it("shows a layered stack and one accessible decision, preserving drafts across navigation and arrivals", async () => {
  const user = userEvent.setup();
  const view = render(<DecisionStack items={items} onEmptyFocus={vi.fn()} />);
  expect(screen.getByRole("region", { name: "3 requests need your decision" })).toBeVisible();
  expect(screen.getByText("1 out of 3")).toBeVisible();
  expect(view.container.querySelectorAll("[data-decision-stack-layer]")).toHaveLength(2);
  expect(screen.getAllByRole("button", { name: /^Allow/ })).toHaveLength(1);
  expect(screen.getByRole("button", { name: "Previous decision" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Next decision" }));
  await user.type(screen.getByRole("textbox", { name: "Draft b" }), "Keep my review");
  view.rerender(<DecisionStack items={[{ id: "new", content: <button>Allow new</button> }, ...items]} onEmptyFocus={vi.fn()} />);
  expect(screen.getByRole("textbox", { name: "Draft b" })).toHaveValue("Keep my review");
  expect(screen.getByText("3 out of 4")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Next decision" }));
  expect(screen.getByRole("button", { name: "Next decision" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Previous decision" }));
  expect(screen.getByRole("textbox", { name: "Draft b" })).toHaveValue("Keep my review");
});

it("keeps requests pinned while collapsed and reveals a deep-linked request", async () => {
  const user = userEvent.setup();
  const view = render(<DecisionStack items={items} onEmptyFocus={vi.fn()} />);
  await user.click(screen.getByRole("button", { name: "Collapse decisions" }));
  expect(screen.queryByRole("button", { name: /^Allow/ })).not.toBeInTheDocument();
  expect(screen.getByRole("region", { name: "3 requests need your decision" })).toBeVisible();
  view.rerender(<DecisionStack items={items} selectedId="c" revealKey="link:c" onEmptyFocus={vi.fn()} />);
  expect(screen.getByRole("button", { name: "Allow c" })).toBeVisible();
  expect(screen.getByText("3 out of 3")).toBeVisible();
});

it("does not move focus from the composer when a request arrives", () => {
  const view = render(<><input aria-label="Composer" /><DecisionStack items={items.slice(0, 1)} onEmptyFocus={vi.fn()} /></>);
  screen.getByRole("textbox", { name: "Composer" }).focus();
  view.rerender(<><input aria-label="Composer" /><DecisionStack items={items} onEmptyFocus={vi.fn()} /></>);
  expect(screen.getByRole("textbox", { name: "Composer" })).toHaveFocus();
});

it("does not reclaim focus when another device resolves a card the user has left", () => {
  const view = render(<><input aria-label="Composer" /><DecisionStack items={items} onEmptyFocus={vi.fn()} /></>);
  screen.getByRole("button", { name: "Allow a" }).focus();
  screen.getByRole("textbox", { name: "Composer" }).focus();
  view.rerender(<><input aria-label="Composer" /><DecisionStack items={items.slice(1)} onEmptyFocus={vi.fn()} /></>);
  expect(screen.getByRole("textbox", { name: "Composer" })).toHaveFocus();
});

it("keeps focus in the composer when a new request arrives after the final decision", async () => {
  const restore = () => screen.getByRole("textbox", { name: "Composer" }).focus();
  const view = render(<><input aria-label="Composer" /><DecisionStack items={items.slice(0, 1)} onEmptyFocus={restore} /></>);
  screen.getByRole("button", { name: "Allow a" }).focus();
  view.rerender(<><input aria-label="Composer" /><DecisionStack items={[]} onEmptyFocus={restore} /></>);
  expect(screen.getByRole("textbox", { name: "Composer" })).toHaveFocus();
  await userEvent.type(screen.getByRole("textbox", { name: "Composer" }), "Still typing");
  view.rerender(<><input aria-label="Composer" /><DecisionStack items={items.slice(1)} onEmptyFocus={restore} /></>);
  expect(screen.getByRole("textbox", { name: "Composer" })).toHaveFocus();
  expect(screen.getByRole("textbox", { name: "Composer" })).toHaveValue("Still typing");
});

it("guards the next decision against a second click and restores focus after the final decision", async () => {
  const decide = vi.fn();
  function Queue() {
    const [pending, setPending] = useState(["a", "b"]);
    const composer = useRef<HTMLInputElement>(null);
    return <><input ref={composer} aria-label="Composer" /><DecisionStack items={pending.map((id) => ({ id, content: <button onClick={() => { decide(id); setPending((rows) => rows.filter((row) => row !== id)); }}>{`Allow ${id}`}</button> }))} onEmptyFocus={() => composer.current?.focus()} /></>;
  }
  render(<Queue />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Allow a" }));
  const next = screen.getByRole("button", { name: "Allow b" });
  expect(next).toBeDisabled();
  await user.click(next);
  expect(decide.mock.calls).toEqual([["a"]]);
  await waitFor(() => expect(next).toBeEnabled());
  await user.click(next);
  expect(decide.mock.calls).toEqual([["a"], ["b"]]);
  expect(screen.getByRole("textbox", { name: "Composer" })).toHaveFocus();
  expect(screen.queryByRole("region")).not.toBeInTheDocument();
});
