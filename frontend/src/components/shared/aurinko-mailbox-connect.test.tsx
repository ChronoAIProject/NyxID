import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AurinkoMailboxConnect } from "./aurinko-mailbox-connect";
import type { AurinkoMailbox } from "@/schemas/aurinko-mailboxes";

const mocks = vi.hoisted(() => ({
  authorize: vi.fn(),
  cancel: vi.fn(),
  list: vi.fn(),
  popup: vi.fn(),
  navigate: vi.fn(),
  close: vi.fn(),
  ack: vi.fn(),
  channel: {
    onmessage: null as ((event: MessageEvent) => void) | null,
    close: vi.fn(),
  },
  mailboxes: [] as AurinkoMailbox[],
}));
vi.mock("@/hooks/use-aurinko-mailboxes", () => ({
  authorizeAurinkoMailbox: mocks.authorize,
  cancelAurinkoAuthorization: mocks.cancel,
  listAurinkoMailboxes: mocks.list,
  useAurinkoMailboxes: () => ({
    data: mocks.mailboxes,
    isPending: false,
    isError: false,
  }),
}));
vi.mock("@/lib/oauth-popup", () => ({
  openOAuthPopup: mocks.popup,
  openOAuthChannel: () => mocks.channel,
  postOAuthAck: mocks.ack,
}));
const nonce = "11111111-1111-4111-8111-111111111111";
const mailbox: AurinkoMailbox = {
  connection_id: "22222222-2222-4222-8222-222222222222",
  service_id: "33333333-3333-4333-8333-333333333333",
  label: "Enterprise email",
  mailbox_address: "test@example.com",
  service_type: "Zoho",
  status: "active",
  is_active: true,
};
const started = {
  connection_id: mailbox.connection_id,
  service_id: mailbox.service_id,
  attempt_nonce: nonce,
  authorization_url: `https://api.aurinko.io/v1/auth/authorize?state=1cc_${nonce}`,
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}
function mount(
  props: Partial<React.ComponentProps<typeof AurinkoMailboxConnect>> = {},
) {
  const onConnected = vi.fn();
  const onAborted = vi.fn();
  const view = render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <AurinkoMailboxConnect
        label="Enterprise email"
        onConnected={onConnected}
        onAborted={onAborted}
        {...props}
      />
    </QueryClientProvider>,
  );
  return { ...view, onConnected, onAborted };
}
async function choose(label: string, option: string) {
  await userEvent.click(screen.getByRole("combobox", { name: label }));
  await userEvent.click(
    screen.getByRole("option", { name: option }),
  );
}
async function start() {
  await userEvent.click(
    screen.getByRole("button", { name: "Connect mailbox" }),
  );
  await waitFor(() => expect(mocks.navigate).toHaveBeenCalled());
}
beforeEach(() => {
  vi.clearAllMocks();
  mocks.mailboxes = [];
  mocks.channel.onmessage = null;
  mocks.authorize.mockResolvedValue(started);
  mocks.cancel.mockResolvedValue(undefined);
  mocks.list.mockResolvedValue([mailbox]);
  mocks.navigate.mockResolvedValue(undefined);
  mocks.popup.mockReturnValue({ navigate: mocks.navigate, close: mocks.close });
});

describe("managed Aurinko mailbox connection", () => {
  it.each([
    ["IMAP / SMTP", "IMAP"],
    ["Zoho Mail", "Zoho"],
  ])(
    "connects %s with the selected owner without collecting mailbox secrets",
    async (label, provider) => {
      mount({ ownerId: "org-1" });
      await choose("Email provider", label);
      expect(
        screen.queryByLabelText(/password|client secret/i),
      ).not.toBeInTheDocument();
      await start();
      expect(mocks.authorize).toHaveBeenCalledWith({
        provider,
        owner_id: "org-1",
        label: "Enterprise email",
      });
    },
  );

  it("reuses an active same-owner mailbox without initiating another authorization", async () => {
    mocks.mailboxes = [mailbox];
    const view = mount({ allowReuse: true, ownerId: "org-1" });
    await choose("Mailbox connection", "test@example.com");
    await userEvent.click(
      screen.getByRole("button", { name: "Use selected mailbox" }),
    );
    await waitFor(() =>
      expect(view.onConnected).toHaveBeenCalledWith(
        mailbox,
        expect.any(AbortSignal),
      ),
    );
    expect(mocks.list).toHaveBeenCalledWith("org-1");
    expect(mocks.authorize).not.toHaveBeenCalled();
    expect(mocks.popup).not.toHaveBeenCalled();
  });

  it("rechecks mailbox availability before reuse", async () => {
    mocks.mailboxes = [mailbox];
    mocks.list.mockResolvedValue([{ ...mailbox, is_active: false }]);
    const view = mount({ allowReuse: true });
    await choose("Mailbox connection", "test@example.com");
    await userEvent.click(
      screen.getByRole("button", { name: "Use selected mailbox" }),
    );
    expect(await screen.findByText(/no longer available/)).toBeInTheDocument();
    expect(view.onConnected).not.toHaveBeenCalled();
  });

  it("locks reconnect to the existing mailbox provider and key", async () => {
    mocks.mailboxes = [mailbox];
    mount({ connectionId: mailbox.connection_id });
    expect(
      screen.getByRole("combobox", { name: "Email provider" }),
    ).toBeDisabled();
    await userEvent.click(
      screen.getByRole("button", { name: "Reconnect mailbox" }),
    );
    await waitFor(() =>
      expect(mocks.authorize).toHaveBeenCalledWith({
        provider: "Zoho",
        label: "Enterprise email",
        connection_id: mailbox.connection_id,
      }),
    );
  });

  it("does not create a fresh mailbox when reconnect identity is missing", () => {
    mount({ connectionId: mailbox.connection_id });
    expect(
      screen.getByRole("button", { name: "Reconnect mailbox" }),
    ).toBeDisabled();
    expect(screen.getByText(/no longer exists/)).toBeInTheDocument();
  });

  it("verifies the completed connection and keeps it when the parent unmounts", async () => {
    const view = mount();
    await start();
    await act(async () =>
      mocks.channel.onmessage?.(
        new MessageEvent("message", {
          data: { type: "oauth_result", status: "complete", flow: "cc" },
        }),
      ),
    );
    await waitFor(() =>
      expect(view.onConnected).toHaveBeenCalledWith(
        mailbox,
        expect.any(AbortSignal),
      ),
    );
    view.unmount();
    expect(mocks.ack).toHaveBeenCalled();
    expect(mocks.cancel).not.toHaveBeenCalled();
    expect(view.onAborted).not.toHaveBeenCalled();
  });

  it("retries bot binding using the authorized mailbox after setup fails", async () => {
    const onConnected = vi.fn().mockRejectedValueOnce(new Error("Subscription setup unavailable")).mockResolvedValue(undefined);
    mount({ allowReuse: true, onConnected });
    await start();
    mocks.mailboxes = [mailbox];
    await act(async () => mocks.channel.onmessage?.(new MessageEvent("message", { data: { type: "oauth_result", status: "complete", flow: "cc" } })));
    expect(await screen.findByText("Subscription setup unavailable")).toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", { name: "Use selected mailbox" }));
    await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(2));
    expect(mocks.authorize).toHaveBeenCalledOnce();
    expect(mocks.cancel).not.toHaveBeenCalled();
  });

  it("ignores completion from another flow", async () => {
    const view = mount();
    await start();
    await act(async () =>
      mocks.channel.onmessage?.(
        new MessageEvent("message", {
          data: { type: "oauth_result", status: "complete", flow: "dc" },
        }),
      ),
    );
    expect(view.onConnected).not.toHaveBeenCalled();
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it("invalidates a delayed authorization response after cancellation", async () => {
    const pending = deferred<typeof started>();
    mocks.authorize.mockReturnValue(pending.promise);
    const view = mount();
    await userEvent.click(
      screen.getByRole("button", { name: "Connect mailbox" }),
    );
    await waitFor(() => expect(mocks.authorize).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await act(async () => pending.resolve(started));
    await waitFor(() => expect(mocks.cancel).toHaveBeenCalledWith(nonce));
    expect(mocks.navigate).not.toHaveBeenCalled();
    expect(view.onConnected).not.toHaveBeenCalled();
    expect(view.onAborted).toHaveBeenCalledWith(nonce);
  });

  it("discards only the prepared pending service when cancelled before authorization", async () => {
    const discardPending = vi.fn().mockResolvedValue(undefined);
    const pending = deferred<{
      connection_id: string;
      service_id: string;
      discardPending: typeof discardPending;
    }>();
    mount({ prepare: () => pending.promise });
    await userEvent.click(
      screen.getByRole("button", { name: "Connect mailbox" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await act(async () => pending.resolve({ ...mailbox, discardPending }));
    await waitFor(() => expect(discardPending).toHaveBeenCalledOnce());
    expect(mocks.authorize).not.toHaveBeenCalled();
  });

  it("retains failed cancellation for explicit retry and blocks duplicate starts", async () => {
    mocks.cancel.mockRejectedValueOnce(new Error("offline"));
    mount();
    await start();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(
      await screen.findByText(/Cancellation could not be confirmed/),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Cancelling..." }));
    expect(mocks.authorize).toHaveBeenCalledOnce();
    await userEvent.click(
      screen.getByRole("button", { name: "Retry cancellation" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Connect mailbox" }),
      ).toBeEnabled(),
    );
    expect(mocks.cancel).toHaveBeenCalledTimes(2);
  });

  it.each([
    "https://attacker.example/authorize",
    `https://api.aurinko.io/unexpected?state=1cc_${nonce}`,
  ])(
    "rejects unsafe authorization URL %s and cancels its nonce",
    async (authorization_url) => {
      mocks.authorize.mockResolvedValue({ ...started, authorization_url });
      mount();
      await userEvent.click(
        screen.getByRole("button", { name: "Connect mailbox" }),
      );
      expect(
        await screen.findByText(/invalid mailbox authorization URL/),
      ).toBeInTheDocument();
      await waitFor(() => expect(mocks.cancel).toHaveBeenCalledWith(nonce));
      expect(mocks.navigate).not.toHaveBeenCalled();
    },
  );

  it("cancels the previous owner attempt and allows a fresh sign-in after switching owners", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const onConnected = vi.fn();
    const content = (ownerId: string) => (
      <QueryClientProvider client={client}>
        <AurinkoMailboxConnect label="Enterprise email" ownerId={ownerId} onConnected={onConnected} />
      </QueryClientProvider>
    );
    const view = render(content("org-1"));
    await start();
    view.rerender(content("org-2"));
    await waitFor(() => expect(screen.getByRole("button", { name: "Connect mailbox" })).toBeEnabled());
    expect(mocks.cancel).toHaveBeenCalledWith(nonce);
    await userEvent.click(screen.getByRole("button", { name: "Connect mailbox" }));
    await waitFor(() => expect(mocks.authorize).toHaveBeenLastCalledWith({ provider: "Google", owner_id: "org-2", label: "Enterprise email" }));
    expect(onConnected).not.toHaveBeenCalled();
  });

  it("invalidates an unfinished attempt on unmount", async () => {
    const view = mount();
    await start();
    view.unmount();
    await waitFor(() => expect(mocks.cancel).toHaveBeenCalledWith(nonce));
    expect(mocks.channel.close).toHaveBeenCalled();
  });
});
