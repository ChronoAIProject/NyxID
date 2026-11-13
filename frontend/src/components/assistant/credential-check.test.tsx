import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { CredentialCheck } from "./credential-check";
import { api } from "@/lib/api-client";
import type { KeyInfo } from "@/types/keys";
import { appI18n } from "@/i18n/instance";
import zh from "@/i18n/locales/zh-CN.json";

vi.mock("@/lib/api-client", () => ({ api: { post: vi.fn() } }));
const key = { id: "own/key", status: "active", credential_type: "api_key", node_id: null, credential_verification: { state: "not_verified", reason_code: "not_checked", can_check: true } } as KeyInfo;
afterEach(async () => { vi.clearAllMocks(); await act(() => appI18n.changeLanguage("en")); });
it("checks only on explicit action and invalidates the saved projection", async () => {
  vi.mocked(api.post).mockResolvedValue({ state: "rejected", reason_code: "provider_unauthorized" });
  const client = new QueryClient(); const invalidate = vi.spyOn(client, "invalidateQueries");
  render(<QueryClientProvider client={client}><CredentialCheck connection={key} canManage /></QueryClientProvider>);
  expect(api.post).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Check credential" }));
  await waitFor(() => expect(api.post).toHaveBeenCalledWith("/keys/own%2Fkey/verify", {}));
  expect(invalidate).toHaveBeenCalledWith({ queryKey: ["keys"] });
});
it("old-server and read-only org connections have no Check action", () => {
  const { rerender } = render(<CredentialCheck connection={{ ...key, credential_verification: undefined }} canManage />);
  expect(screen.queryByRole("button")).toBeNull();
  rerender(<CredentialCheck connection={key} canManage={false} />);
  expect(screen.queryByRole("button")).toBeNull();
});
it("renders Chinese copy and permission-aware rejection", async () => {
  appI18n.addResourceBundle("zh-CN", "app", zh, true, true);
  await act(() => appI18n.changeLanguage("zh-CN"));
  render(<CredentialCheck connection={{ ...key, credential_verification: { state: "rejected", reason_code: "provider_forbidden", can_check: true } }} canManage />);
  expect(screen.getByText(zh["The provider refused this check. Review the credential's permissions; this does not necessarily mean the key is invalid."])).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "检查凭据" })).toBeInTheDocument();
});

it("reports a failed check without inventing provider rejection", async () => {
  vi.mocked(api.post).mockRejectedValue(new Error("offline"));
  render(<CredentialCheck connection={key} canManage />);
  await userEvent.click(screen.getByRole("button", { name: "Check credential" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not check the credential");
  expect(screen.queryByText("Rejected by provider")).toBeNull();
});
