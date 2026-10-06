import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { ThreadTitle } from "./thread-title";

it("edits inline with focus, a dirty-gated save and escaped title text", async () => {
  const user = userEvent.setup();
  const rename = vi.fn().mockResolvedValue(undefined);
  render(<ThreadTitle title="<b>Old title</b>" onRename={rename} />);
  expect(screen.getByRole("button", { name: "Rename chat" })).toHaveTextContent(
    "<b>Old title</b>",
  );
  await user.click(screen.getByRole("button", { name: "Rename chat" }));
  const input = screen.getByRole("textbox", { name: "Chat title" });
  expect(input).toHaveFocus();
  expect(screen.getByRole("button", { name: "Save title" })).toBeDisabled();
  await user.clear(input);
  await user.type(input, "My title{Enter}");
  await waitFor(() => expect(rename).toHaveBeenCalledWith("My title"));
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
});

it("allows cancelling and keeps a failed edit available to retry", async () => {
  const user = userEvent.setup();
  const rename = vi.fn().mockRejectedValue(new Error("failed"));
  render(<ThreadTitle title="Old title" onRename={rename} />);
  await user.click(screen.getByRole("button", { name: "Rename chat" }));
  await user.type(screen.getByRole("textbox"), " changed{Enter}");
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Could not rename",
  );
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
});
