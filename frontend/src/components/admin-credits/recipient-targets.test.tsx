import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { zodResolver } from "@hookform/resolvers/zod";
import { describe, expect, it, vi } from "vitest";
import { Form, useAppForm } from "@/components/ui/form";
import {
  allowanceBundleFormSchema as allowanceFormSchema,
  issueGrantFormSchema,
  scheduleFormSchema,
  type AllowanceBundleForm as AllowanceForm,
  type IssueGrantForm,
  type ScheduleForm,
} from "@/schemas/billing-credits";
import { GrantDialog, AllowanceDialog } from "./credits-dialogs";
import { ScheduleDialog } from "./schedule-dialog";
import { RecipientTargetFields } from "./recipient-targets";

vi.mock("@/hooks/use-admin", () => ({
  useAdminUsers: () => ({
    data: {
      users: [
        {
          id: "org",
          display_name: "Engineering",
          slug: "engineering",
          is_active: true,
        },
      ],
    },
    isFetching: false,
    isError: false,
  }),
}));
vi.mock("@/hooks/use-rbac", () => ({
  useGroups: () => ({
    data: {
      groups: [
        {
          id: "group",
          name: "Developers",
          slug: "developers",
          member_count: 4,
        },
      ],
    },
    isFetching: false,
    isError: false,
  }),
}));

function Harness({
  benefit,
  onSubmit,
}: {
  readonly benefit: "shared" | "grant" | "allowance" | "schedule";
  readonly onSubmit: () => Promise<void>;
}) {
  const targets = {
    target_kind: "all_users" as const,
    target_user_ids: [],
    target_org_ids: [],
    target_group_ids: [],
  };
  const grant = useAppForm<IssueGrantForm>({
    resolver: zodResolver(issueGrantFormSchema),
    defaultValues: {
      ...targets,
      amount_credits: 10,
      all_services: true,
      service_refs: [],
      expires_at: "",
      reason: "",
    },
  });
  const allowance = useAppForm<AllowanceForm>({
    resolver: zodResolver(allowanceFormSchema),
    defaultValues: {
      ...targets,
      service_ref: "service",
      units: [{ metric: "tokens", quantity: 100, recurrence: "monthly" }],
    },
  });
  const schedule = useAppForm<ScheduleForm>({
    resolver: zodResolver(scheduleFormSchema),
    defaultValues: {
      ...targets,
      amount_credits: 10,
      recurrence: "monthly",
      expiry: { kind: "never" },
      all_services: true,
      service_refs: [],
      reason: "",
    },
  });
  const props = {
    open: true,
    onOpenChange: vi.fn(),
    services: [],
    pending: false,
    onSubmit,
  };
  if (benefit === "shared") {
    return (
      <Form {...grant}>
        <form onSubmit={grant.handleSubmit(onSubmit)}>
          <RecipientTargetFields />
          <button type="submit">Submit recipients</button>
        </form>
      </Form>
    );
  }
  if (benefit === "grant") return <GrantDialog {...props} form={grant} />;
  if (benefit === "allowance")
    return (
      <AllowanceDialog {...props} form={allowance} editingAllowance={null} />
    );
  return <ScheduleDialog {...props} form={schedule} editingSchedule={null} />;
}

describe.each([
  ["shared", "Submit recipients"],
  ["grant", "Issue credits"],
  ["allowance", "Create allowance"],
  ["schedule", "Create schedule"],
] as const)("%s member recipients", (benefit, submitLabel) => {
  it("submits organizations, clears old lists on switching and submits groups", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(<Harness benefit={benefit} onSubmit={onSubmit} />);
    await user.click(screen.getByRole("combobox", { name: "Recipients" }));
    await user.click(
      screen.getByRole("option", { name: "Organization members" }),
    );
    expect(screen.getByText(/including viewers/)).toBeInTheDocument();
    await user.click(screen.getByText("Engineering"));
    await user.click(screen.getByRole("button", { name: submitLabel }));
    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith(
        expect.objectContaining({
          target_kind: "org_members",
          target_user_ids: [],
          target_org_ids: ["org"],
          target_group_ids: [],
        }),
        expect.anything(),
      ),
    );
    onSubmit.mockClear();
    await user.click(screen.getByRole("combobox", { name: "Recipients" }));
    await user.click(screen.getByRole("option", { name: "Group members" }));
    await user.click(screen.getByText("Developers"));
    await user.click(screen.getByRole("button", { name: submitLabel }));
    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith(
        expect.objectContaining({
          target_kind: "groups",
          target_user_ids: [],
          target_org_ids: [],
          target_group_ids: ["group"],
        }),
        expect.anything(),
      ),
    );
    onSubmit.mockClear();
    await user.click(screen.getByRole("combobox", { name: "Recipients" }));
    await user.click(
      screen.getByRole("option", { name: "All billing owners" }),
    );
    await user.click(screen.getByRole("button", { name: submitLabel }));
    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith(
        expect.objectContaining({
          target_kind: "all_users",
          target_user_ids: [],
          target_org_ids: [],
          target_group_ids: [],
        }),
        expect.anything(),
      ),
    );
  });
});

it("shared fields clear selected owners when switching to member targets", async () => {
  const onSubmit = vi.fn().mockResolvedValue(undefined);
  const user = userEvent.setup();
  render(<Harness benefit="shared" onSubmit={onSubmit} />);
  await user.click(screen.getByRole("combobox", { name: "Recipients" }));
  await user.click(screen.getByRole("option", { name: "Selected owners" }));
  expect(screen.getByText(/its shared wallet/)).toBeInTheDocument();
  await user.click(screen.getByText("Engineering"));
  await user.click(screen.getByRole("button", { name: "Submit recipients" }));
  await waitFor(() =>
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        target_kind: "selected_users",
        target_user_ids: ["org"],
      }),
      expect.anything(),
    ),
  );
  onSubmit.mockClear();
  await user.click(screen.getByRole("combobox", { name: "Recipients" }));
  await user.click(
    screen.getByRole("option", { name: "Organization members" }),
  );
  await user.click(screen.getByText("Engineering"));
  await user.click(screen.getByRole("button", { name: "Submit recipients" }));
  await waitFor(() =>
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        target_kind: "org_members",
        target_user_ids: [],
        target_org_ids: ["org"],
      }),
      expect.anything(),
    ),
  );
});
