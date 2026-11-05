import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it } from "vitest";
import { SectionInfo } from "./section-info";
afterEach(cleanup);
it("opens on keyboard focus and closes on Escape without dismissing its parent", async () => {
  const user = userEvent.setup();
  render(<SectionInfo title="Services">Choose which connected services this specialist may call.</SectionInfo>);
  await user.tab();
  expect(screen.getByRole("button", { name: "About Services" })).toHaveFocus();
  expect(await screen.findByRole("tooltip", { name: "About Services" })).toHaveTextContent("Choose which connected services");
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
});
it("opens from a touch tap and supports dismissal", async () => {
  const user = userEvent.setup();
  render(<SectionInfo title="Memory">Notes remembered across chats.</SectionInfo>);
  const button = screen.getByRole("button", { name: "About Memory" });
  await user.pointer([{ keys: "[TouchA>]", target: button }, { keys: "[/TouchA]", target: button }]);
  expect(await screen.findByRole("tooltip", { name: "About Memory" })).toHaveTextContent("Notes remembered across chats.");
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
});
