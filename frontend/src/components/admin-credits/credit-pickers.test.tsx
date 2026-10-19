import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { DownstreamService } from "@/types/api";
import { useAdminUsers } from "@/hooks/use-admin";
import { GroupPicker, OrgPicker, ServicePicker } from "./credit-pickers";

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

it("keeps missing and filtered saved services visible and removable", () => {
  const onChange = vi.fn();
  render(
    <ServicePicker
      services={[service("old", "Old service", "requests")]}
      selected={["old", "missing"]}
      onChange={onChange}
      multiple
    />,
  );
  fireEvent.change(screen.getByPlaceholderText("Search services"), {
    target: { value: "no matches" },
  });
  expect(
    screen.getByLabelText(
      "Selected service: Old service (outside current results)",
    ),
  ).toBeChecked();
  fireEvent.click(
    screen.getByLabelText(
      "Selected service: missing (outside current results)",
    ),
  );
  expect(onChange).toHaveBeenCalledWith(["old"]);
});

vi.mock("@/hooks/use-admin", () => ({
  useAdminUsers: vi.fn(() => ({
    data: {
      users: [
        {
          id: "org-1",
          display_name: "Engineering",
          slug: "engineering",
          email: "org@example.com",
          is_active: true,
        },
        {
          id: "inactive",
          display_name: "Disabled organization",
          slug: "disabled",
          is_active: false,
        },
      ],
    },
    isFetching: false,
    isError: false,
  })),
}));
vi.mock("@/hooks/use-rbac", () => ({
  useGroups: vi.fn(() => ({
    data: {
      groups: [
        {
          id: "group-1",
          name: "Developers",
          slug: "developers",
          member_count: 12,
        },
      ],
    },
    isFetching: false,
    isError: false,
  })),
}));

it("searches active organizations by slug and retains off-result selections", () => {
  const onChange = vi.fn();
  render(<OrgPicker selected={["org-1", "missing"]} onChange={onChange} />);
  expect(useAdminUsers).toHaveBeenLastCalledWith(1, 100, undefined, "org");
  expect(screen.getByText("engineering")).toBeInTheDocument();
  expect(screen.queryByText("Disabled organization")).not.toBeInTheDocument();
  expect(screen.getByText("2 selected")).toBeInTheDocument();
  fireEvent.change(screen.getByPlaceholderText("Search organizations"), {
    target: { value: "no matches" },
  });
  expect(useAdminUsers).toHaveBeenLastCalledWith(1, 100, "no matches", "org");
  expect(
    screen.getByLabelText(
      "Selected organization: Engineering (outside current results)",
    ),
  ).toBeChecked();
  fireEvent.click(
    screen.getByLabelText(
      "Selected organization: missing (outside current results)",
    ),
  );
  expect(onChange).toHaveBeenCalledWith(["org-1"]);
});

it("shows group name, slug and member count with searchable, removable selections", () => {
  const onChange = vi.fn();
  const { rerender } = render(
    <GroupPicker selected={[]} onChange={onChange} />,
  );
  expect(screen.getByText("developers · 12 members")).toBeInTheDocument();
  fireEvent.click(screen.getByText("Developers"));
  expect(onChange).toHaveBeenCalledWith(["group-1"]);
  rerender(
    <GroupPicker selected={["group-1", "missing"]} onChange={onChange} />,
  );
  fireEvent.change(screen.getByPlaceholderText("Search groups"), {
    target: { value: "no matches" },
  });
  expect(
    screen.getByLabelText(
      "Selected group: Developers (outside current results)",
    ),
  ).toBeChecked();
  expect(screen.getByText("2 selected")).toBeInTheDocument();
  fireEvent.click(
    screen.getByLabelText("Selected group: missing (outside current results)"),
  );
  expect(onChange).toHaveBeenCalledWith(["group-1"]);
});
