import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { WhatsappNativeSection } from "./whatsapp-native";
import { ApiError } from "@/lib/api-client";

const mocks = vi.hoisted(() => ({
  settings: { enabled: true, reason: null as string | null, meta_app_id: "111", waba_id: "222", phone_number_id: "333", dedicated_app: true, revision: 2, callback_phase: "relay", callback_error: null as string | null },
  mutate: vi.fn(),
  error: null as Error | null,
}));
vi.mock("@/hooks/use-whatsapp-native", () => ({
  useWhatsappNative: () => ({ data: mocks.settings, isError: mocks.error !== null, error: mocks.error }),
  useSaveWhatsappNative: () => ({ mutate: mocks.mutate, isPending: false, isError: false }),
  useReplaceWhatsappRelayToken: () => ({ mutate: vi.fn(), reset: vi.fn(), isPending: false, isError: false }),
}));

describe("native WhatsApp settings", () => {
  beforeEach(() => {
    mocks.settings.enabled = true; mocks.settings.reason = null; mocks.settings.callback_phase = "relay";
    mocks.mutate.mockClear();
    mocks.error = null;
  });
  it("saves distinct app/WABA identity with the revision fence", async () => {
    render(<WhatsappNativeSection botId="bot" />);
    fireEvent.change(screen.getByLabelText("Meta App ID"), { target: { value: "444" } });
    await waitFor(() => expect(screen.getByRole("button", { name: "Save identity" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Save identity" }));
    await waitFor(() => expect(mocks.mutate).toHaveBeenCalledWith({ meta_app_id: "444", waba_id: "222", dedicated_app: true, revision: 2 }, expect.anything()));
  });
  it("hides the dormant settings", () => {
    mocks.settings.enabled = false;
    render(<WhatsappNativeSection botId="bot" />);
    expect(screen.queryByRole("button", { name: "Save identity" })).not.toBeInTheDocument();
  });
  it.each([403, 404])("keeps the existing page unchanged for an unavailable optional panel (%s)", (status) => {
    mocks.error = new ApiError(status, { error: "unavailable", error_code: 0, message: "Unavailable" });
    const view = render(<WhatsappNativeSection botId="bot" />);
    expect(view.container).toBeEmptyDOMElement();
  });
  it("explains managed ineligibility without allowing edits", () => {
    mocks.settings.reason = "dedicated_byo_app_required";
    render(<WhatsappNativeSection botId="bot" />);
    expect(screen.getByText(/Managed Embedded Signup bots/)).toBeInTheDocument();
    expect(screen.queryByLabelText("Meta App ID")).not.toBeInTheDocument();
  });
  it("shows pending callback restoration even with the flag off", () => {
    mocks.settings.enabled = false; mocks.settings.callback_phase = "awaiting_relay";
    render(<WhatsappNativeSection botId="bot" />);
    expect(screen.getByRole("status")).toHaveTextContent("Restore the NyxID relay callback");
    expect(screen.getByRole("button", { name: "Save identity" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Generate replacement Verify Token" })).toBeEnabled();
  });
});
