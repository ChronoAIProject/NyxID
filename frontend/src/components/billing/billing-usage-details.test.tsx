import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AllowanceFunding, ServiceUsage } from "./billing-usage-details";
import { billingRow } from "@/test/billing-fixture";

describe("retained and permanent usage", () => {
  it("renders separate recent and older lines while counting one service", () => {
    const recent = billingRow({ service_slug: "svc", service_id: "svc", detail_retained: true, lago_acked: false, events: 2 });
    const older = billingRow({ ...recent, detail_retained: false, events: 3 });
    const { container } = render(<ServiceUsage catalog={[]} rows={[recent, older]} />);
    expect(container.querySelectorAll("details")).toHaveLength(2);
    expect(Array.from(container.querySelectorAll(".usage-status"), (node) => node.textContent)).toEqual(["Pending", "Unavailable"]);
    expect(screen.getByText("svc · Older than 30 days")).toBeInTheDocument();
    expect(container.querySelector("h4 span")?.textContent).toBe("1 service");
  });
  it("does not display zero allowance units when older units were not retained", () => {
    render(<AllowanceFunding rows={[billingRow({ detail_retained: false, allowance_quantity: 0 })]} />);
    expect(screen.getByText("Unavailable")).toBeInTheDocument();
  });
});
