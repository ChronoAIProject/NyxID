import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { useChangeReview } from "./change-review-dialog";

it("discards a pending review on a source switch, including a switch back", () => {
  const save = vi.fn();
  function Editor({ sourceId }: { sourceId: string }) {
    const review = useChangeReview(save, false, sourceId);
    return (
      <>
        {review.dialog}
        <button
          onClick={() =>
            review.review({ id: sourceId, name: "Draft" }, [
              { field: "Name", before: "Original", after: "Draft" },
            ])
          }
        >
          Review
        </button>
      </>
    );
  }
  const view = render(<Editor sourceId="a" />);
  fireEvent.click(screen.getByText("Review"));
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  view.rerender(<Editor sourceId="b" />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  view.rerender(<Editor sourceId="a" />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(save).not.toHaveBeenCalled();
});
