import { useState } from "react";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { GroupedFormActions, GroupedFormBar, GroupedFormScope } from "./grouped-form";

afterEach(cleanup);

function Draft({ name, initiallyDirty = false, save }: { name: string; initiallyDirty?: boolean; save: () => Promise<void> }) {
  const [dirty, setDirty] = useState(initiallyDirty);
  return <>
    <button onClick={() => setDirty(true)}>Edit {name}</button>
    <GroupedFormActions label={name} dirty={dirty} discard={() => setDirty(false)} save={async () => { await save(); setDirty(false); }} />
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
  expect(first).toHaveBeenCalledOnce();
  await userEvent.click(screen.getByRole("button", { name: "Edit second" }));
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
  await userEvent.click(screen.getByRole("button", { name: "Remove second" }));
  await act(async () => release());
  expect(second).not.toHaveBeenCalled();
  await waitFor(() => expect(screen.queryByRole("region", { name: "Unsaved changes" })).toBeNull());
});
