vi.mock("./composer-editor", () => import("./composer-editor.test-support"));
vi.mock("./composer-settings", () => ({
  ComposerSettings: () => <span>Permissions</span>,
}));
const state = vi.hoisted(() => ({
  data: [] as import("@/types/keys").KeyInfo[],
  error: false,
}));
vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    data: state.error ? undefined : state.data,
    isError: state.error,
    isPending: false,
  }),
}));
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NyxAgentComposer } from "./nyxagent-composer";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import type { KeyInfo } from "@/types/keys";

const connection = {
  id: "org-connection",
  label: "Work documents",
  slug: "work-drive",
  is_active: true,
  catalog_service_id: "google",
  catalog_service_name: "Google Drive",
  credential_source: {
    type: "org",
    org_name: "Team",
    org_id: "org-team",
    allowed: true,
  },
} as KeyInfo;
beforeEach(() => {
  state.data = [connection];
  state.error = false;
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
});
function mount(onSend = vi.fn().mockResolvedValue(undefined)) {
  const query = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const props = {
    scope: { kind: "conversations" as const, id: "selection-test" },
    ownerUserId: "owner",
    draftKey: "selection:test",
    active: false,
    sending: false,
    onSend,
    onStop: vi.fn().mockResolvedValue(undefined),
  };
  const view = render(
    <QueryClientProvider client={query}>
      <NyxAgentComposer {...props} />
    </QueryClientProvider>,
  );
  return {
    onSend,
    rerender: (draftKey = props.draftKey, ownerUserId = props.ownerUserId) =>
      view.rerender(
        <QueryClientProvider client={query}>
          <NyxAgentComposer
            {...props}
            draftKey={draftKey}
            ownerUserId={ownerUserId}
          />
        </QueryClientProvider>,
      ),
  };
}
async function openPicker() {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Add context" }));
  await user.click(screen.getByRole("menuitem", { name: "Use a connection" }));
  return user;
}
async function choose() {
  const user = await openPicker();
  await user.click(
    screen.getByRole("button", { name: /Work documents.*Team/ }),
  );
}
function submit(text: string) {
  fireEvent.change(screen.getByRole("textbox"), { target: { value: text } });
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
}
describe("visible one-message connection context", () => {
  it("shows the exact account, sends a newline selector and clears only after success", async () => {
    const view = mount();
    await choose();
    expect(
      screen.getByRole("group", { name: "Connection for this message" }),
    ).toHaveTextContent("Google Drive · Work documents · Team · This message");
    submit("Use this account");
    await waitFor(() =>
      expect(view.onSend).toHaveBeenCalledWith(
        "/service org-connection\nUse this account",
        undefined,
        undefined,
      ),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("group", { name: "Connection for this message" }),
      ).not.toBeInTheDocument(),
    );
    submit("Another operation");
    await waitFor(() =>
      expect(view.onSend).toHaveBeenLastCalledWith(
        "Another operation",
        undefined,
        undefined,
      ),
    );
  });
  it("retains the exact account after a failed send", async () => {
    const send = vi
      .fn()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue(undefined);
    mount(send);
    await choose();
    submit("Keep my account");
    await waitFor(() => expect(send).toHaveBeenCalledOnce());
    expect(
      screen.getByRole("group", { name: "Connection for this message" }),
    ).toHaveTextContent("Work documents");
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Send message" }),
      ).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(send).toHaveBeenCalledTimes(2));
    expect(send.mock.calls[0]?.[0]).toBe(send.mock.calls[1]?.[0]);
  });
  it("lets the owner remove the override and return to saved order", async () => {
    const view = mount();
    await choose();
    fireEvent.click(
      screen.getByRole("button", {
        name: "Remove connection for this message",
      }),
    );
    submit("Use defaults");
    await waitFor(() =>
      expect(view.onSend).toHaveBeenCalledWith(
        "Use defaults",
        undefined,
        undefined,
      ),
    );
  });
  it.each(["revoked", "read failed"])(
    "refuses sending when the selected account is %s",
    async (reason) => {
      const view = mount();
      await choose();
      if (reason === "revoked") state.data = [];
      else state.error = true;
      view.rerender();
      submit("Do not switch accounts");
      await waitFor(() =>
        expect(
          screen.getByRole("group", { name: "Connection for this message" }),
        ).toHaveTextContent("Unavailable"),
      );
      expect(view.onSend).not.toHaveBeenCalled();
    },
  );
  it("resets connection context when navigating to another draft", async () => {
    const view = mount();
    await choose();
    view.rerender("selection:other");
    await waitFor(() =>
      expect(
        screen.queryByRole("group", { name: "Connection for this message" }),
      ).not.toBeInTheDocument(),
    );
  });
  it("shows disabled and unauthorized accounts with reasons and searches by account", async () => {
    state.data = [
      connection,
      {
        ...connection,
        id: "disabled",
        label: "Disabled account",
        is_active: false,
      },
      {
        ...connection,
        id: "denied",
        label: "Denied account",
        credential_source: {
          type: "org",
          org_name: "Other",
          org_id: "other",
          allowed: false,
          role: "viewer",
        },
      },
    ];
    mount();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Add context" }));
    await user.click(
      screen.getByRole("menuitem", { name: "Use a connection" }),
    );
    expect(
      screen.getByRole("button", { name: /Disabled account/ }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: /Disabled account/ }),
    ).toHaveAccessibleDescription("Disabled");
    expect(
      screen.getByRole("button", { name: /Denied account/ }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: /Denied account/ }),
    ).toHaveAccessibleDescription("No access");
    await user.type(
      screen.getByRole("textbox", { name: "Search connections" }),
      "no such account",
    );
    expect(screen.getByText("No matching connections.")).toBeVisible();
  });

  it("updates an open picker and skips unavailable options for pointer and keyboard selection", async () => {
    state.data = [
      connection,
      {
        ...connection,
        id: "personal",
        label: "Personal documents",
        credential_source: { type: "personal" },
      },
    ];
    const view = mount();
    const user = await openPicker();
    const work = screen.getByRole("button", { name: /Work documents/ });
    work.focus();
    state.data = [{ ...connection, credential_missing: true }, state.data[1]!];
    view.rerender();
    expect(work).toBeDisabled();
    expect(work).toHaveAccessibleDescription("Credential missing");
    await user.click(work);
    await user.keyboard("{Enter} ");
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(
      screen.queryByRole("group", { name: "Connection for this message" }),
    ).not.toBeInTheDocument();
    screen.getByRole("textbox", { name: "Search connections" }).focus();
    await user.tab();
    expect(
      screen.getByRole("button", { name: /Personal documents/ }),
    ).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    submit("Use personal");
    await waitFor(() =>
      expect(view.onSend).toHaveBeenCalledWith(
        "/service personal\nUse personal",
        undefined,
        undefined,
      ),
    );
  });

  it.each<Partial<KeyInfo>>([
    { credential_binding: "platform", platform_key_available: false },
    { credential_missing: true },
    { status: "refresh_failed" },
    {
      node_id: "node",
      node_status: "offline",
      execution_unavailable_reason: "node home offline",
    },
  ])(
    "retains the selected ID and refuses both send paths after live invalidation: %j",
    async (update) => {
      const view = mount();
      await choose();
      state.data = [{ ...connection, ...update }];
      view.rerender();
      expect(screen.getByRole("status")).toHaveTextContent(
        "Unavailable — choose another connection or remove",
      );
      submit("Do not switch accounts");
      await waitFor(() =>
        expect(screen.getByRole("textbox")).toHaveValue(
          "Do not switch accounts",
        ),
      );
      fireEvent.keyDown(screen.getByRole("textbox"), {
        key: "Enter",
        code: "Enter",
      });
      await waitFor(() =>
        expect(screen.getByRole("textbox")).toHaveValue(
          "Do not switch accounts",
        ),
      );
      expect(view.onSend).not.toHaveBeenCalled();
      expect(
        screen.getByRole("group", { name: "Connection for this message" }),
      ).toHaveTextContent("Work documents");
    },
  );

  it("renders the live label and owner and only changes account on explicit selection", async () => {
    const view = mount();
    await choose();
    state.data = [
      {
        ...connection,
        label: "Updated documents",
        credential_source: { type: "personal" },
      },
      { ...connection, id: "another", label: "Another account" },
    ];
    view.rerender();
    const chip = screen.getByRole("group", {
      name: "Connection for this message",
    });
    expect(chip).toHaveTextContent("Updated documents · Personal");
    expect(chip).not.toHaveTextContent("Work documents");
    const user = await openPicker();
    await user.click(screen.getByRole("button", { name: /Another account/ }));
    submit("Switch explicitly");
    await waitFor(() =>
      expect(view.onSend).toHaveBeenCalledWith(
        "/service another\nSwitch explicitly",
        undefined,
        undefined,
      ),
    );
  });

  it("clears the selection on signed-in account change", async () => {
    const view = mount();
    await choose();
    view.rerender("selection:test", "another-owner");
    expect(
      screen.queryByRole("group", { name: "Connection for this message" }),
    ).not.toBeInTheDocument();
  });
});
