import { act, render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { appI18n } from "@/i18n";
import type { OrgChannelPreview } from "@/schemas/org-specialist-channel";
import { OrgChannelDisclosure } from "./org-channel-preview";

it("describes a closed inbox in each language and only lists capabilities when guests have access", async () => {
  const preview: OrgChannelPreview = {
    bot_id: "bot", agent_id: "specialist", org_id: "org", guest_enabled: false,
    private_chats: "owner", guest_policy_mode: "legacy",
    description: "Customer access starts off. Only verified organization members can use private chats until an administrator enables customer conversations.",
    effective_guest_services: [{ id: "calendar", label: "Calendar", access: "read", limited_operations: false }],
    audience_readiness: { guest_readiness: { status: "not_ready" }, note: "" },
  };
  const view = render(<OrgChannelDisclosure preview={preview} />);
  try {
    expect(screen.getByText(preview.description)).toBeVisible();
    expect(screen.queryByText(/Calendar/)).toBeNull();
    expect(screen.queryByText("Guests can use")).toBeNull();
    await act(async () => { await appI18n.changeLanguage("zh-CN"); });
    expect(screen.getByText(/客户访问默认为关闭/)).toBeVisible();
    await act(async () => { await appI18n.changeLanguage("zh-TW"); });
    expect(screen.getByText(/客戶存取權預設為關閉/)).toBeVisible();
    view.rerender(<OrgChannelDisclosure preview={{ ...preview, guest_enabled: true, description: "Open" }} />);
    expect(screen.getByText(/Calendar/)).toBeVisible();
  } finally {
    view.unmount();
    await appI18n.changeLanguage("en");
  }
});
