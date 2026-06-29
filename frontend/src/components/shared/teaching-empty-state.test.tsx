import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Boxes } from "lucide-react";
import { TeachingEmptyState } from "./teaching-empty-state";

describe("TeachingEmptyState", () => {
  it("renders title, description, icon and primary CTA", () => {
    const onClick = vi.fn();
    render(
      <TeachingEmptyState
        icon={Boxes}
        title="No services yet"
        description="Connect an API service to start brokering credentials."
        primaryCta={{ label: "Add service", onClick }}
      />,
    );

    expect(screen.getByText("No services yet")).toBeInTheDocument();
    expect(
      screen.getByText("Connect an API service to start brokering credentials."),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Add service" }),
    ).toBeInTheDocument();
  });

  it("fires the onClick handler when the actionable CTA is clicked", async () => {
    const onClick = vi.fn();
    const user = userEvent.setup();
    render(
      <TeachingEmptyState
        icon={Boxes}
        title="t"
        description="d"
        primaryCta={{ label: "Go", onClick }}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Go" }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("renders a link when primaryCta.href is provided", () => {
    render(
      <TeachingEmptyState
        icon={Boxes}
        title="t"
        description="d"
        primaryCta={{ label: "Docs", href: "/docs" }}
      />,
    );
    const link = screen.getByRole("link", { name: "Docs" });
    expect(link).toHaveAttribute("href", "/docs");
  });

  it("hides the jump-start row when catalogJumpStarts is omitted or empty", () => {
    const { rerender } = render(
      <TeachingEmptyState
        icon={Boxes}
        title="t"
        description="d"
        primaryCta={{ label: "Go", onClick: vi.fn() }}
      />,
    );
    expect(screen.queryByText(/Or start with:/i)).not.toBeInTheDocument();

    rerender(
      <TeachingEmptyState
        icon={Boxes}
        title="t"
        description="d"
        primaryCta={{ label: "Go", onClick: vi.fn() }}
        catalogJumpStarts={[]}
      />,
    );
    expect(screen.queryByText(/Or start with:/i)).not.toBeInTheDocument();
  });

  it("renders jump-starts capped at 5 even when more are passed", () => {
    const onClick = vi.fn();
    render(
      <TeachingEmptyState
        icon={Boxes}
        title="t"
        description="d"
        primaryCta={{ label: "Go", onClick: vi.fn() }}
        catalogJumpStarts={Array.from({ length: 8 }, (_, i) => ({
          label: `Start ${String(i)}`,
          onClick,
        }))}
      />,
    );
    expect(screen.getByText("Or start with:")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start 0" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start 4" })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Start 5" }),
    ).not.toBeInTheDocument();
  });
});
