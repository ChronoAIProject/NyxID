import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { expect } from "vitest";

/** An explicit second click proves edits are reviewed before the existing write. */
export async function confirmChanges(waitForClose = true) {
  const review = await screen.findByRole("dialog", { name: "Review changes" });
  fireEvent.click(within(review).getByRole("button", { name: "Save changes" }));
  if (waitForClose) await waitFor(() => expect(review).not.toBeInTheDocument());
}
