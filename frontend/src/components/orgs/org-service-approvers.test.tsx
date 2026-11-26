import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api, ApiError } from "@/lib/api-client";
import type { ServiceApprovers } from "@/schemas/service-approvers";
import { useAuthStore } from "@/stores/auth-store";
import { OrgServiceApprovers } from "./org-service-approvers";

const alice = "00000000-0000-4000-8000-000000000001";
const bob = "00000000-0000-4000-8000-000000000002";
const retired = "00000000-0000-4000-8000-000000000003";
const inherited: ServiceApprovers = {
  config_id: "config-one", revision: 2, approver_user_ids: null, authoring_available: true,
  eligible_candidates: [{ user_id: alice, display_name: "Alice" }, { user_id: bob, display_name: null }],
};
const path = "/approvals/service-configs/catalog/approvers?org_id=org";
const row = <OrgServiceApprovers orgId="org" serviceId="catalog" serviceName="Reports" />;

beforeEach(() => {
  useAuthStore.setState({ user: { id: alice, email: "", display_name: null, avatar_url: null,
    email_verified: true, mfa_enabled: false, is_admin: false, is_active: true, created_at: "" } });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); useAuthStore.setState({ user: null }); });

async function open(policy: ServiceApprovers, extraRows?: React.ReactNode) {
  const get = vi.spyOn(api, "get").mockResolvedValue(policy);
  const put = vi.spyOn(api, "put");
  const user = userEvent.setup();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<QueryClientProvider client={client}>{row}{extraRows}</QueryClientProvider>);
  await user.click(await screen.findByRole("button", { name: "Approvers for Reports" }));
  async function save() {
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    const review = await screen.findByRole("dialog", { name: "Review changes" });
    await user.click(within(review).getByRole("button", { name: "Save changes" }));
  }
  return { user, get, put, save };
}

it("keeps the named draft on conflict until reload, then uses the replacement config and revision", async () => {
  const { user, get, put, save } = await open(inherited);
  expect(screen.getByRole("radio", { name: "All eligible Admins" })).toBeChecked();
  await user.click(screen.getByRole("radio", { name: "Selected Admins" }));
  expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
  await user.click(screen.getByRole("checkbox", { name: "Alice" }));
  put.mockRejectedValueOnce(new ApiError(409, { error: "conflict", error_code: 409, message: "stale" }));
  await save();
  await screen.findByRole("button", { name: "Reload saved" });
  expect(put).toHaveBeenCalledWith(path, { config_id: "config-one", expected_revision: 2, approver_user_ids: [alice] }, expect.anything());
  expect(screen.getByRole("checkbox", { name: "Alice" })).toBeChecked();
  expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
  expect(get).toHaveBeenCalledTimes(1);
  const replacement = { ...inherited, config_id: "config-two", revision: 0, approver_user_ids: [bob] };
  get.mockResolvedValue(replacement);
  await user.click(screen.getByRole("button", { name: "Reload saved" }));
  await waitFor(() => expect(screen.getByRole("checkbox", { name: bob })).toBeChecked());
  expect(screen.getByRole("checkbox", { name: "Alice" })).not.toBeChecked();
  await user.click(screen.getByRole("radio", { name: "All eligible Admins" }));
  put.mockResolvedValueOnce({ ...replacement, revision: 1, approver_user_ids: null });
  await save();
  expect(put).toHaveBeenCalledTimes(2);
  expect(put).toHaveBeenLastCalledWith(path, { config_id: "config-two", expected_revision: 0, approver_user_ids: null }, expect.anything());
});

it("retains unavailable selections until removed and saves only server candidates", async () => {
  const { user, save, put, get } = await open({ ...inherited, approver_user_ids: [retired] });
  const unavailable = screen.getByRole("checkbox", { name: retired + " Unavailable" });
  expect(unavailable).toBeChecked();
  await user.click(screen.getByRole("checkbox", { name: "Alice" }));
  expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
  await user.click(unavailable);
  put.mockResolvedValueOnce({ ...inherited, revision: 3, approver_user_ids: [alice] });
  await save();
  expect(get).toHaveBeenCalledWith(path, expect.anything());
  expect(put).toHaveBeenCalledWith(path, { config_id: "config-one", expected_revision: 2, approver_user_ids: [alice] }, expect.anything());
});

it.each([{ ids: [] }, { ids: [retired] }])("shows the saved read-only restriction and empty eligibility ($ids)", async ({ ids }) => {
  const { put } = await open({ ...inherited, approver_user_ids: ids, authoring_available: false, eligible_candidates: [] });
  expect(screen.getByText("Editing is unavailable")).toBeVisible();
  expect(screen.getByText("No Admins are eligible for this service.")).toBeVisible();
  expect(screen.getByRole("radio", { name: "Selected Admins" })).toBeChecked();
  expect(screen.getByRole("radio", { name: "All eligible Admins" })).toBeDisabled();
  if (ids.length) expect(screen.getByRole("checkbox", { name: retired + " Unavailable" })).toBeDisabled();
  else expect(screen.getByText(/No Admins are selected/)).toBeVisible();
  expect(screen.queryByRole("button", { name: "Save changes" })).toBeNull();
  expect(put).not.toHaveBeenCalled();
});

it("shows the saved policy when authoring closes during an edit", async () => {
  const { user, get, put, save } = await open({ ...inherited, approver_user_ids: [bob] });
  await user.click(screen.getByRole("checkbox", { name: "Alice" }));
  put.mockRejectedValueOnce(new ApiError(403, { error: "forbidden", error_code: 1, message: "Unavailable" }));
  get.mockResolvedValue({ ...inherited, approver_user_ids: [bob], authoring_available: false });
  await save();
  await screen.findByText("Editing is unavailable");
  expect(screen.getByRole("checkbox", { name: bob })).toBeChecked();
  expect(screen.queryByRole("checkbox", { name: "Alice" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Save changes" })).toBeNull();
  expect(put).toHaveBeenCalledTimes(1);
});

it("scopes each policy fetch by organization and service", async () => {
  const { get } = await open(inherited, <>
    <OrgServiceApprovers orgId="org" serviceId="custom" serviceName="Calendar" />
    <OrgServiceApprovers orgId="second" serviceId="catalog" serviceName="Archive" />
  </>);
  expect(get.mock.calls.map(([url]) => url).sort()).toEqual([
    path, "/approvals/service-configs/custom/approvers?org_id=org",
    "/approvals/service-configs/catalog/approvers?org_id=second",
  ].sort());
});

it("allows replacing one of 128 selections and searching the candidate list", async () => {
  const people = Array.from({ length: 129 }, (_, i) => ({
    user_id: crypto.randomUUID(), display_name: `Admin ${i}`,
  }));
  const { user } = await open({ ...inherited, eligible_candidates: people, approver_user_ids: people.slice(0, 128).map((p) => p.user_id) });
  const search = screen.getByRole("searchbox");
  await user.type(search, "Admin 128");
  expect(screen.getByRole("checkbox", { name: "Admin 128" })).toBeDisabled();
  await user.clear(search);
  await user.click(screen.getByRole("checkbox", { name: "Admin 0" }));
  await user.type(search, "Admin 128");
  await user.click(screen.getByRole("checkbox", { name: "Admin 128" }));
  expect(screen.getByRole("button", { name: "Save changes" })).toBeEnabled();
});
