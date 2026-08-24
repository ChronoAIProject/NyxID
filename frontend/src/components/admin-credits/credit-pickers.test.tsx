import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { DownstreamService } from "@/types/api";
import { ServicePicker } from "./credit-pickers";

function service(
  id: string,
  name: string,
  metric: DownstreamService["effective_platform_metric"],
): DownstreamService {
  return {
    id,
    name,
    slug: id,
    is_active: true,
    effective_platform_metric: metric,
  } as DownstreamService;
}

describe("ServicePicker", () => {
  it("shows backend-resolved metrics and filters service rows", () => {
    render(
      <ServicePicker
        services={[
          service("llm-one", "Token service", "tokens"),
          service("ssh-one", "Byte service", "bytes"),
        ]}
        selected={[]}
        onChange={vi.fn()}
      />,
    );

    expect(screen.getByText("tokens")).toBeInTheDocument();
    expect(screen.getByText("bytes")).toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText("Search services"), {
      target: { value: "byte" },
    });
    expect(screen.queryByText("Token service")).not.toBeInTheDocument();
    expect(screen.getByText("Byte service")).toBeInTheDocument();
  });

  it("replaces the selected service in single-select mode", () => {
    const onChange = vi.fn();
    render(
      <ServicePicker
        services={[service("service-one", "Service one", "requests")]}
        selected={[]}
        onChange={onChange}
      />,
    );

    fireEvent.click(screen.getByText("Service one"));
    expect(onChange).toHaveBeenCalledWith(["service-one"]);
  });
});
