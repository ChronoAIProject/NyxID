vi.mock("./composer-editor", () => import("./composer-editor.test-support"));
vi.mock("./composer-settings", () => ({
  ComposerSettings: () => <span>Permissions</span>,
}));
const state = vi.hoisted(() => ({
  data: [] as import("@/types/keys").KeyInfo[],
  error: false,
  fetching: false,
  paused: false,
  refetch: vi.fn(),
}));
vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    data: state.error ? undefined : state.data,
    isError: state.error,
    isPending: false,
    isFetching: state.fetching,
    isPaused: state.paused,
    refetch: state.refetch,
  }),
}));
vi.mock("@/hooks/use-current-keys", () => ({
  useCurrentKeysReader: () => () => ({
    authority: undefined,
    data: state.error ? undefined : state.data,
    isError: state.error,
    isRefreshing: state.fetching || state.paused,
    fetchStatus: state.paused ? "paused" : state.fetching ? "fetching" : "idle",
  }),
}));
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "sonner";
import { appI18n } from "@/i18n/instance";
import zhCN from "@/i18n/locales/app/zh-CN";
import zhTW from "@/i18n/locales/app/zh-TW";
import { translateUi } from "@/i18n/use-ui-translation";
import { markAdmitted } from "@/lib/assistant/send-admission";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
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
  transitionAssistantIdentity("owner");
  state.data = [connection];
  state.error = false;
  state.fetching = false;
  state.paused = false;
  state.refetch.mockClear();
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
    rerender: (draftKey = props.draftKey, ownerUserId = props.ownerUserId, disabled = false) =>
      view.rerender(
        <QueryClientProvider client={query}>
          <NyxAgentComposer
            {...props}
            draftKey={draftKey}
            ownerUserId={ownerUserId}
            disabled={disabled}
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
      if (reason === "revoked") expect(screen.getByRole("group", { name: "Connection for this message" })).toHaveTextContent("No longer listed");
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
    ).toHaveAttribute("aria-disabled", "true");
    expect(
      screen.getByRole("button", { name: /Disabled account/ }),
    ).toHaveAccessibleDescription("Disabled");
    expect(
      screen.getByRole("button", { name: /Denied account/ }),
    ).toHaveAttribute("aria-disabled", "true");
    expect(
      screen.getByRole("button", { name: /Denied account/ }),
    ).toHaveAccessibleDescription("No access");
    await user.type(
      screen.getByRole("textbox", { name: "Search connections" }),
      "no such account",
    );
    expect(screen.getByText("No matching connections.")).toBeVisible();
  });

  it("updates an open picker and reaches unavailable options for pointer and keyboard selection", async () => {
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
    expect(work).toHaveAttribute("aria-disabled", "true");
    expect(work).toHaveAccessibleDescription("Credential missing");
    await user.click(work);
    await user.keyboard("{Enter} ");
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(
      screen.queryByRole("group", { name: "Connection for this message" }),
    ).not.toBeInTheDocument();
    screen.getByRole("textbox", { name: "Search connections" }).focus();
    await user.tab();
    expect(work).toHaveFocus();
    await user.keyboard("{Enter} ");
    expect(screen.getByRole("dialog")).toBeVisible();
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

it("refreshes exactly once per opening, joins an in-flight fetch and keeps cached rows visible", async () => {
  const view = mount();
  await openPicker();
  expect(state.refetch).toHaveBeenCalledOnce();
  expect(screen.getByRole("button", { name: /Work documents/ })).toHaveAttribute("aria-disabled", "false");
  state.fetching = true;
  state.data = [{ ...connection, credential_missing: true }];
  view.rerender();
  expect(screen.getByRole("button", { name: /Work documents/ })).toHaveAttribute("aria-disabled", "true");
  expect(screen.queryByText("Loading connections…")).not.toBeInTheDocument();
  await userEvent.setup().keyboard("{Escape}");
  await openPicker();
  expect(state.refetch).toHaveBeenCalledOnce();
  state.error = true;
  view.rerender();
  expect(screen.getByRole("alert")).toHaveTextContent("Could not load connections.");
  await userEvent.setup().click(screen.getByRole("button", { name: "Retry" }));
  expect(state.refetch).toHaveBeenCalledTimes(2);
});

it("closes an open picker permanently when a lock begins", async () => {
  const view = mount();
  await openPicker();
  view.rerender("selection:test", "owner", true);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  view.rerender();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("changes the chip and send ID only after an explicit replacement", async () => {
  const view = mount();
  await choose();
  state.data = [{ ...connection, credential_missing: true }, { ...connection, id: "replacement", label: "Replacement" }];
  view.rerender();
  await userEvent.setup().click(screen.getByRole("button", { name: "Change connection for this message" }));
  await userEvent.setup().click(screen.getByRole("button", { name: /Replacement/ }));
  expect(screen.getByRole("group", { name: "Connection for this message" })).toHaveTextContent("Replacement");
  submit("Use replacement");
  await waitFor(() => expect(view.onSend).toHaveBeenCalledWith("/service replacement\nUse replacement", undefined, undefined));
});

it("updates both open option and selected chip at expiry, preserving OAuth refreshability", async () => {
  const view = mount();
  await choose();
  const start = Date.now();
  vi.useFakeTimers();
  try {
    vi.setSystemTime(start);
    state.data = [
      { ...connection, credential_type: "api_key", expires_at: new Date(start + 1000).toISOString() },
      { ...connection, id: "oauth", label: "OAuth", credential_type: "oauth2", expires_at: new Date(start + 1000).toISOString() },
    ];
    view.rerender();
    fireEvent.click(screen.getByRole("button", { name: "Change connection for this message" }));
    expect(screen.getByRole("button", { name: /Work documents/ })).toHaveAttribute("aria-disabled", "false");
    act(() => vi.advanceTimersByTime(1000));
    expect(screen.getByRole("button", { name: /Work documents/ })).toHaveAccessibleDescription("Credential expired");
    expect(screen.getByRole("button", { name: /Work documents/ })).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("button", { name: /OAuth/ })).toHaveAttribute("aria-disabled", "false");
    expect(screen.getByRole("group", { name: "Connection for this message", hidden: true })).toHaveTextContent("Credential expired");
  } finally { vi.useRealTimers(); }
});

it("updates the same refusal toast after click and Enter attempts", async () => {
  const error = vi.spyOn(toast, "error").mockReturnValue("refused");
  try {
    const view = mount();
    await choose();
    state.data = [{ ...connection, credential_missing: true }];
    view.rerender();
    submit("Keep this draft");
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter", code: "Enter" });
    expect(error).toHaveBeenCalledTimes(2);
    const ids = error.mock.calls.map((call) => call[1]?.id);
    expect(ids[0]).toBeTruthy();
    expect(ids[1]).toBe(ids[0]);
    expect(screen.getByRole("textbox")).toHaveValue("Keep this draft");
    expect(view.onSend).not.toHaveBeenCalled();
  } finally { error.mockRestore(); }
});

it.each(["api_key", "oauth2"])("checks wall-clock expiry after throttled timers for %s", async (credentialType) => {
  useAssistantDraftStore.getState().saveDraft("owner", "selection:test", "Keep this persisted draft");
  const view = mount();
  await choose();
  const start = Date.now();
  vi.useFakeTimers();
  try {
    vi.setSystemTime(start);
    state.data = [{ ...connection, credential_type: credentialType, expires_at: new Date(start + 1000).toISOString() }];
    view.rerender();
    const chip = screen.getByRole("group", { name: "Connection for this message" });
    expect(chip).not.toHaveTextContent("Credential expired");
    // Jump wall time without firing the expiry timer, as in a background tab.
    vi.setSystemTime(start + 2000);
    expect(chip).not.toHaveTextContent("Credential expired");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    });
    if (credentialType === "api_key") {
      expect(view.onSend).not.toHaveBeenCalled();
      expect(screen.getByRole("textbox")).toHaveValue("Keep this persisted draft");
      expect(useAssistantDraftStore.getState().getDraft("selection:test")).toBe("Keep this persisted draft");
      expect(chip).toHaveTextContent("Credential expired");
    } else {
      expect(view.onSend).toHaveBeenCalledWith("/service org-connection\nKeep this persisted draft", undefined, undefined);
      expect(screen.queryByRole("group", { name: "Connection for this message" })).not.toBeInTheDocument();
    }
  } finally { vi.useRealTimers(); }
});

it.each([
  ["zh-CN", zhCN, "所选连接不可用。请在发送前选择其他连接或移除此连接。"],
  ["zh-TW", zhTW, "所選連線無法使用。請在傳送前選擇其他連線或移除此連線。"],
] as const)("localizes the composer refusal toast through the app catalog in %s", async (language, catalog, expected) => {
  const error = vi.spyOn(toast, "error").mockReturnValue("refused");
  try {
    const view = mount();
    await choose();
    state.data = [{ ...connection, credential_missing: true }];
    view.rerender();
    appI18n.addResourceBundle(language, "app", catalog, true, true);
    await act(() => appI18n.changeLanguage(language));
    const message = "The selected connection is unavailable. Choose another connection or remove it before sending.";
    expect(translateUi(message)).toBe(expected);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "Keep this draft" } });
    fireEvent.click(screen.getByRole("button", { name: translateUi("Send message") }));
    expect(error).toHaveBeenCalledExactlyOnceWith(expected, {
      id: "composer-connection-unavailable",
      description: translateUi("Credential missing"),
    });
    expect(screen.getByRole("textbox")).toHaveValue("Keep this draft");
    expect(view.onSend).not.toHaveBeenCalled();
  } finally {
    error.mockRestore();
    await act(() => appI18n.changeLanguage("en"));
  }
});

it("A2: consumes the override after an admitted interruption before the next message", async () => {
  const admitted = markAdmitted(new Error("stream interrupted"));
  const send = vi.fn().mockRejectedValueOnce(admitted).mockResolvedValue(undefined);
  mount(send);
  await choose();
  submit("First message");
  await waitFor(() => expect(send).toHaveBeenCalledOnce());
  expect(screen.getByRole("textbox")).toHaveValue("");
  expect(screen.queryByRole("group", { name: "Connection for this message" })).not.toBeInTheDocument();
  submit("Different message");
  await waitFor(() => expect(send).toHaveBeenCalledTimes(2));
  expect(send.mock.calls.map((call) => call[0])).toEqual([
    "/service org-connection\nFirst message", "Different message",
  ]);
});

it.each([
  ["en", "Checking the selected connection. Send again when it's ready."],
  ["zh-CN", "正在检查所选连接。就绪后请再次发送。"],
  ["zh-TW", "正在檢查所選連線。就緒後請再次傳送。"],
] as const)("R6-4: distinguishes checking and unavailable refusal toasts in %s", async (language, checkingTitle) => {
  const error = vi.spyOn(toast, "error").mockReturnValue("refused");
  try {
    const view = mount(); await choose();
    appI18n.addResourceBundle("zh-CN", "app", zhCN, true, true);
    appI18n.addResourceBundle("zh-TW", "app", zhTW, true, true);
    await act(() => appI18n.changeLanguage(language));
    state.fetching = true; view.rerender();
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "Keep this draft" } });
    fireEvent.click(screen.getByRole("button", { name: translateUi("Send message") }));
    expect(error).toHaveBeenCalledExactlyOnceWith(checkingTitle, {
      id: "composer-connection-unavailable", description: translateUi("Checking availability…"),
    });
    state.fetching = false; state.data = [{ ...connection, credential_missing: true }]; view.rerender();
    fireEvent.click(screen.getByRole("button", { name: translateUi("Send message") }));
    expect(error).toHaveBeenLastCalledWith(translateUi("The selected connection is unavailable. Choose another connection or remove it before sending."), {
      id: "composer-connection-unavailable", description: translateUi("Credential missing"),
    });
    expect(screen.getByRole("textbox")).toHaveValue("Keep this draft");
    expect(view.onSend).not.toHaveBeenCalled();
  } finally {
    error.mockRestore();
    await act(() => appI18n.changeLanguage("en"));
  }
});

it.each(["click", "Enter", "Space"])("A3: refuses %s activation at wall-clock expiry without timer delivery", async (method) => {
  const start = Date.now();
  state.data = [{ ...connection, credential_type: "api_key", expires_at: new Date(start + 60000).toISOString() }];
  mount();
  await openPicker();
  const clock = vi.spyOn(Date, "now").mockReturnValue(start + 70000);
  try {
    const option = screen.getByRole("button", { name: /Work documents/ });
    if (method === "click") fireEvent.click(option);
    else { option.focus(); await userEvent.setup().keyboard(method === "Enter" ? "{Enter}" : " "); }
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(option).toHaveAccessibleDescription("Credential expired");
    expect(screen.queryByRole("group", { name: "Connection for this message" })).not.toBeInTheDocument();
  } finally { clock.mockRestore(); }
});
it.each([
  { credential_type: "oauth2" },
  { credential_type: "api_key", node_id: "node", node_status: "offline", execution_unavailable_reason: null },
])("A3: retains refresh and node fallback eligibility at activation: %j", async (patch) => {
  const start = Date.now();
  state.data = [{ ...connection, ...patch, expires_at: new Date(start + 60000).toISOString() } as KeyInfo];
  mount(); await openPicker();
  const clock = vi.spyOn(Date, "now").mockReturnValue(start + 70000);
  try {
    fireEvent.click(screen.getByRole("button", { name: /Work documents/ }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Connection for this message" })).toHaveTextContent("This message");
  } finally { clock.mockRestore(); }
});

it("A1: another activation replaces a hold; navigation cancels it and clears search", async () => {
  state.data = [connection, { ...connection, id: "second", label: "Second account" }];
  state.fetching = true;
  const view = mount();
  const user = await openPicker();
  await user.click(screen.getByRole("button", { name: /Work documents/ }));
  expect(screen.getByRole("button", { name: /Work documents/ })).toHaveAttribute("aria-busy", "true");
  await user.click(screen.getByRole("button", { name: /Second account/ }));
  expect(screen.getByRole("button", { name: /Second account/ })).toHaveAttribute("aria-busy", "true");
  state.fetching = false; view.rerender();
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(screen.getByRole("group", { name: "Connection for this message" })).toHaveTextContent("Second account");
  state.fetching = true; view.rerender();
  await openPicker();
  await user.type(screen.getByRole("textbox", { name: "Search connections" }), "Work");
  await user.click(screen.getByRole("button", { name: /Work documents/ }));
  view.rerender("selection:other");
  state.fetching = false; view.rerender("selection:other");
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByRole("group", { name: "Connection for this message" })).not.toBeInTheDocument();
  await openPicker();
  expect(screen.getByRole("textbox", { name: "Search connections" })).toHaveValue("");
  expect(screen.getByRole("button", { name: /Second account/ })).toBeVisible();
});
