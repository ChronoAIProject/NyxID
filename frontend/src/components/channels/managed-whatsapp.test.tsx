import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { expect, it, vi } from "vitest";
import { appI18n } from "@/i18n";
import { ManagedWhatsApp } from "./managed-whatsapp";

vi.mock("@/lib/meta-embedded-signup", () => ({
  loadFacebookSdk: async () => ({ init: vi.fn(), login: vi.fn() }),
  parseEmbeddedSignupEvent: vi.fn(),
}));
vi.mock("@/hooks/use-runtime-config", () => ({
  useRuntimeConfig: () => ({ data: {}, error: null }),
}));
vi.mock("@/hooks/use-channel-connect-link", () => ({
  useChannelConnectLinkContext: () => null,
}));

it("updates WhatsApp signup choices with the app language while preserving the selected flow", async () => {
  const user = userEvent.setup();
  const client = new QueryClient();
  const onConnected = vi.fn();
  const view = render(
    <QueryClientProvider client={client}>
      <ManagedWhatsApp
        bootstrap={{ available: true, app_id: "123", graph_version: "v24.0", required_scopes: [], signup_extras: {}, feature_types: ["", "whatsapp_business_app_onboarding"] }}
        label="Customer support"
        orgId={null}
        onConnected={onConnected}
      />
    </QueryClientProvider>,
  );
  try {
    await act(async () => { await appI18n.changeLanguage("zh-CN"); });
    expect(screen.getByRole("radio", { name: "新的或现有的 Cloud API 号码" })).toBeChecked();
    const existing = screen.getByRole("radio", { name: "我已在该号码上使用 WhatsApp Business 应用程序" });
    await user.click(existing);
    expect(existing).toBeChecked();
    await act(async () => { await appI18n.changeLanguage("en"); });
    expect(screen.getByRole("radio", { name: "I already use the WhatsApp Business app on this number" })).toBeChecked();
    expect(onConnected).not.toHaveBeenCalled();
  } finally {
    view.unmount();
    client.clear();
    await appI18n.changeLanguage("en");
  }
});
