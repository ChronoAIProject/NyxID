import { confirmChanges } from "@/test-helpers/confirm-changes";
import { useState } from "react";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { GroupedFormActions, GroupedFormBar, GroupedFormScope } from "./grouped-form";
import { toast } from "sonner";

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

function Draft({ name, initiallyDirty = false, save }: { name: string; initiallyDirty?: boolean; save: () => Promise<void> }) {
  const [dirty, setDirty] = useState(initiallyDirty);
  return <>
    <button onClick={() => setDirty(true)}>Edit {name}</button>
    <GroupedFormActions label={name} dirty={dirty} changes={() => dirty ? [{ label: "Enabled", before: "Off", after: "On" }] : []} discard={() => setDirty(false)} save={async () => { await save(); setDirty(false); }} />
  </>;
}

it("uses a primary submit, ghost Discard and an opaque separated dirty-only bar", async () => {
  render(<GroupedFormScope><Draft name="profile" save={vi.fn()} /><GroupedFormBar /></GroupedFormScope>);
  expect(screen.queryByRole("region", { name: "Unsaved changes" })).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Edit profile" }));
  const bar = screen.getByRole("region", { name: "Unsaved changes" });
  expect(bar).toHaveClass("border-t", "border-border/50", "bg-background");
  expect(screen.getByRole("button", { name: "Save changes" })).toHaveClass("nyx-gradient-vivid");
  expect(screen.getByRole("button", { name: "Discard" })).not.toHaveClass("nyx-gradient-vivid");
});

it("saves a second form dirtied while the first request awaits in the same pass", async () => {
  let release!: () => void;
  const first = vi.fn(() => new Promise<void>((resolve) => { release = resolve; }));
  const second = vi.fn().mockResolvedValue(undefined);
  render(<GroupedFormScope><Draft name="first" initiallyDirty save={first} /><Draft name="second" save={second} /><GroupedFormBar /></GroupedFormScope>);
  await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges(false);
  expect(first).toHaveBeenCalledOnce();
  act(() => screen.getByText("Edit second").click());
  await act(async () => release());
  await waitFor(() => expect(second).toHaveBeenCalledOnce());
  expect(screen.queryByRole("region", { name: "Unsaved changes" })).toBeNull();
});

it("skips a form removed while an earlier request awaits", async () => {
  let release!: () => void;
  const first = vi.fn(() => new Promise<void>((resolve) => { release = resolve; }));
  const second = vi.fn().mockResolvedValue(undefined);
  function Forms() {
    const [show, setShow] = useState(true);
    return <GroupedFormScope><button onClick={() => setShow(false)}>Remove second</button><Draft name="first" initiallyDirty save={first} />{show && <Draft name="second" initiallyDirty save={second} />}<GroupedFormBar /></GroupedFormScope>;
  }
  render(<Forms />);
  await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges(false);
  act(() => screen.getByText("Remove second").click());
  await act(async () => release());
  expect(second).not.toHaveBeenCalled();
  await waitFor(() => expect(screen.queryByRole("region", { name: "Unsaved changes" })).toBeNull());
});

it("reviews each dirty section before saving and names only successes after a partial failure", async () => {
  const success = vi.spyOn(toast, "success");
  const profile = vi.fn().mockResolvedValue(undefined);
  const grants = vi.fn().mockRejectedValue(new Error("Revision changed"));
  render(<GroupedFormScope viewport><Draft name="profile" initiallyDirty save={profile} /><Draft name="grants" initiallyDirty save={grants} /><GroupedFormBar viewport /></GroupedFormScope>);
  await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
  expect(screen.getByRole("dialog", { name: "Review changes" })).toHaveTextContent("Before: Off");
  expect(screen.getByRole("dialog", { name: "Review changes" })).toHaveTextContent("After: On");
  expect(screen.getByRole("heading", { name: "Profile" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "Grants" })).toBeVisible();
  expect(profile).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Keep editing" }));
  expect(profile).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(() => expect(success).toHaveBeenCalledWith("Saved profile"));
  expect(grants).toHaveBeenCalledOnce();
  expect(screen.getByRole("region", { name: "Unsaved changes" })).toHaveTextContent("Unsaved: grants");
  await userEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(screen.queryByRole("region", { name: "Unsaved changes" })).toBeNull();
  success.mockRestore();
});

it("keeps both values visible when a long previous value is truncated in review", async () => {
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(48);
  vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(function (this: HTMLElement) { return (this.textContent?.length ?? 0) > 160 ? 120 : 24; });
  const before = "Previous instructions ".repeat(30), after = "Revised instructions ".repeat(30);
  render(<GroupedFormActions label="profile" dirty changes={() => [{ label: "Persona", before, after }]} save={vi.fn()} discard={vi.fn()} />);
  await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
  const previous = screen.getByText(before.trim());
  const revised = screen.getByText(after.trim());
  expect(previous.parentElement).toHaveClass("line-clamp-3");
  expect(previous).toHaveClass("line-through");
  expect(previous.closest("[data-review-value]")).toHaveClass("text-text-tertiary");
  expect(revised.parentElement).toHaveClass("line-clamp-3");
  expect(revised).not.toHaveClass("line-through");
  expect(revised.closest("[data-review-value]")).toHaveClass("text-foreground");
  expect(screen.queryByText("→")).toBeNull();
  await userEvent.click(within(previous.closest<HTMLElement>("[data-review-value]")!).getByRole("button", { name: "Show more" }));
  expect(previous.parentElement).not.toHaveClass("line-clamp-3");
  expect(revised.parentElement).toHaveClass("line-clamp-3");
  expect(screen.getByRole("button", { name: "Show less" })).toHaveAttribute("aria-expanded", "true");
});

it("uses sentence-case section labels and separate list additions and removals", async () => {
  render(<GroupedFormActions label="machine access" dirty changes={() => [{ label: "Machines", added: ["Office Mac"], removed: ["Old laptop"] }]} save={vi.fn()} discard={vi.fn()} />);
  await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
  expect(screen.getByRole("heading", { name: "Machine access" })).toBeVisible();
  expect(screen.getByText("+ Office Mac")).toBeVisible();
  expect(screen.getByText("− Old laptop")).toBeVisible();
  expect(screen.queryByText("Before:")).toBeNull();
});
