import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { GuestReadiness } from "./guest-readiness";

describe("guest model readiness", () => {
  it("does not invent readiness for an older server", () => {
    const { container } = render(<GuestReadiness />);
    expect(container.textContent).toBe("");
  });
  it("explains ready model and owner billing without widening services", () => {
    render(<GuestReadiness readiness={{ status: "ready" }} />);
    expect(screen.getByText("Model ready for visitors")).toBeTruthy();
    expect(screen.getByText(/Service permissions still apply.*billed to you/)).toBeTruthy();
  });
  it("shows the exact owner fix and keeps diagnostic codes out of the copy", () => {
    const { container } = render(<GuestReadiness readiness={{ status: "not_ready", reason: "connection_not_platform", fix: "Send one message to this assistant yourself." }} />);
    expect(screen.getByText("Visitor setup needs attention")).toBeTruthy();
    expect(screen.getByText("Send one message to this assistant yourself.")).toBeTruthy();
    expect(container.textContent).not.toContain("connection_not_platform");
  });
});
