import { beforeEach, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { NativePlatformEntrySetup } from "./native-platform-entry-setup";
const fixture = vi.hoisted(() => ({
  actor: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  convert: vi.fn(),
  inspect: vi.fn(),
  lifecycle: vi.fn(),
  bots: [
    {
      id: "11111111-1111-4111-8111-111111111111",
      label: "Discord help",
      platform: "discord",
    },
    {
      id: "22222222-2222-4222-8222-222222222222",
      label: "X help",
      platform: "x",
    },
  ],
  orgs: [
    {
      id: "33333333-3333-4333-8333-333333333333",
      display_name: "Help funding",
    },
    {
      id: "44444444-4444-4444-8444-444444444444",
      display_name: "Other funding",
    },
  ],
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (s: unknown) => unknown) =>
    select({ user: { id: fixture.actor } }),
}));
vi.mock("@/hooks/use-platform-onboarding", () => ({
  useAdminPlatformTargets: () => ({
    data: { bots: fixture.bots, orgs: fixture.orgs },
  }),
}));
vi.mock("@/hooks/use-native-platform-entry", () => ({
  useNativeLifecycleReview: () => ({
    mutate: fixture.lifecycle,
    isPending: false,
  }),
  useNativeEntryTransports: () => ({
    data: [
      { platform: "discord", transports: ["direct", "gateway"] },
      { platform: "x", transports: ["direct"] },
    ],
  }),
  useNativeEntryConversion: () => ({
    mutate: fixture.convert,
    isPending: false,
  }),
  useNativeGatewayInspection: () => ({
    mutate: fixture.inspect,
    isPending: false,
  }),
}));
beforeEach(() => {
  fixture.convert.mockClear();
  fixture.inspect.mockClear();
  fixture.lifecycle.mockClear();
  fixture.actor = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
});
function choose(platform: "discord" | "x", transport?: string) {
  fireEvent.change(screen.getByLabelText("Native chat channel"), {
    target: {
      value: fixture.bots.find((bot) => bot.platform === platform)!.id,
    },
  });
  fireEvent.change(screen.getByLabelText("Native funding organization"), {
    target: { value: fixture.orgs[0]!.id },
  });
  if (transport)
    fireEvent.change(screen.getByLabelText("Native delivery transport"), {
      target: { value: transport },
    });
}
it("offers server-returned transports and requires an explicit direct choice for X", () => {
  render(<NativePlatformEntrySetup />);
  choose("x");
  expect(
    screen.queryByRole("option", { name: "Gateway transport" }),
  ).not.toBeInTheDocument();
  const submit = screen.getByRole("button", {
    name: "Convert native entry and pause",
  });
  expect(submit).toBeDisabled();
  expect(fixture.convert).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText("Native delivery transport"), {
    target: { value: "direct" },
  });
  fireEvent.click(submit);
  expect(fixture.convert.mock.calls[0]![0]).toEqual({
    channel_bot_id: fixture.bots[1]!.id,
    funding_owner_user_id: fixture.orgs[0]!.id,
    transport: "direct",
  });
});
it("inspects retained gateway setup and submits only the reviewed exact snapshot", () => {
  const page = render(<NativePlatformEntrySetup />);
  choose("discord", "gateway");
  fireEvent.click(
    screen.getByRole("button", { name: "Inspect retained gateway setup" }),
  );
  expect(fixture.inspect.mock.calls[0]![0]).toBe(fixture.bots[0]!.id);
  expect(fixture.convert).not.toHaveBeenCalled();
  const review = {
    kind: "gateway",
    record_id: "55555555-5555-4555-8555-555555555555",
    attempt_id: "66666666-6666-4666-8666-666666666666",
    revision: 1,
    request_hash: `sha256:${"a".repeat(64)}`,
    credential_fingerprint: "b".repeat(64),
    remote_channel_id: null,
    remote_version: null,
    remote_ready: false,
  };
  act(() => fixture.inspect.mock.calls[0]![1].onSuccess(review));
  const retry = screen.getByRole("button", {
    name: "Retry reviewed native setup",
  });
  expect(retry).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(retry);
  expect(fixture.convert.mock.calls[0]![0].recovery).toEqual(review);
  fixture.actor = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
  page.rerender(<NativePlatformEntrySetup />);
  expect(
    screen.queryByRole("button", { name: "Retry reviewed native setup" }),
  ).not.toBeInTheDocument();
});
it("drops a confirmed review when the selected funding or transport changes", () => {
  render(<NativePlatformEntrySetup />);
  choose("discord", "gateway");
  fireEvent.click(
    screen.getByRole("button", { name: "Convert native entry and pause" }),
  );
  act(() =>
    fixture.convert.mock.calls[0]![1].onSuccess({
      status: "recovery",
      review: { kind: "discord", application_id: "123", attempt_id: "attempt" },
    }),
  );
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.change(screen.getByLabelText("Native funding organization"), {
    target: { value: fixture.orgs[1]!.id },
  });
  expect(
    screen.queryByRole("button", { name: "Retry reviewed native setup" }),
  ).not.toBeInTheDocument();
});

it("requires exact lifecycle review and resets it on credential or funding change", () => {
  const page = render(<NativePlatformEntrySetup />);
  choose("discord", "direct");
  fireEvent.change(screen.getByLabelText("Native verification public key"), {
    target: { value: "a".repeat(64) },
  });
  fireEvent.click(
    screen.getByRole("button", {
      name: "Review failed or changed native setup",
    }),
  );
  const review = {
    channel_bot_id: fixture.bots[0]!.id,
    profile_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
    generation: null,
    reviewed_by: fixture.actor,
    snapshot_digest: "c".repeat(64),
  };
  act(() => fixture.lifecycle.mock.calls[0]![1].onSuccess(review));
  const submit = screen.getByRole("button", {
    name: "Update reviewed native entry and pause",
  });
  expect(submit).toBeDisabled();
  fireEvent.click(
    screen.getByRole("checkbox", {
      name: "I reviewed this entry and its selected replacement credentials.",
    }),
  );
  fireEvent.click(submit);
  expect(fixture.convert.mock.calls[0]![0]).toMatchObject({
    lifecycle_review: review,
    public_key: "a".repeat(64),
    transport: "direct",
  });
  fireEvent.change(screen.getByLabelText("Native verification public key"), {
    target: { value: "b".repeat(64) },
  });
  page.rerender(<NativePlatformEntrySetup />);
  expect(
    screen.queryByRole("button", {
      name: "Update reviewed native entry and pause",
    }),
  ).not.toBeInTheDocument();
});

it("an existing native profile remains selectable for a second review after successful update", () => {
  const profile = {
    id: "55555555-5555-4555-8555-555555555555",
    channel_bot_id: fixture.bots[0]!.id,
    manager_bot_id: "discord-application",
    execution_profile: "native_entry" as const,
    platform: "discord" as const,
    transport: "direct" as const,
    funding_owner_user_id: fixture.orgs[0]!.id,
    enabled: false,
    generation: 1,
    deleted_at: null,
    guide: null,
  };
  const page = render(<NativePlatformEntrySetup profile={profile} />);
  fireEvent.change(screen.getByLabelText("Native delivery transport"), {
    target: { value: "direct" },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Review current native entry" }),
  );
  const review = {
    channel_bot_id: profile.channel_bot_id,
    profile_id: profile.id,
    generation: 1,
    reviewed_by: fixture.actor,
    snapshot_digest: "c".repeat(64),
  };
  act(() => fixture.lifecycle.mock.calls[0]![1].onSuccess(review));
  fireEvent.click(
    screen.getByRole("checkbox", {
      name: "I reviewed this entry and its selected replacement credentials.",
    }),
  );
  fireEvent.click(
    screen.getByRole("button", {
      name: "Update reviewed native entry and pause",
    }),
  );
  expect(fixture.convert).toHaveBeenCalledTimes(1);
  act(() =>
    fixture.convert.mock.calls[0]![1].onSuccess({
      status: "converted",
      profile: { ...profile, generation: 2 },
    }),
  );
  page.rerender(
    <NativePlatformEntrySetup profile={{ ...profile, generation: 2 }} />,
  );
  expect(screen.getByLabelText("Native chat channel")).toHaveValue(
    profile.channel_bot_id,
  );
  fireEvent.change(screen.getByLabelText("Native delivery transport"), {
    target: { value: "direct" },
  });
  expect(
    screen.getByRole("button", { name: "Review current native entry" }),
  ).toBeEnabled();
});

it("recovers the authoritative saved profile when the successor has an empty target picker", () => {
  const savedBots = [...fixture.bots];
  fixture.bots.length = 0;
  try {
    const profile = {
      id: "profile",
      generation: 4,
      channel_bot_id: "saved-channel",
      manager_bot_id: "remote-bot",
      display_name: "Saved help bot",
      platform: "discord",
      transport: "direct",
      supported_transports: ["direct", "gateway"],
    } as import("@/schemas/platform-onboarding").PlatformProfile;
    render(<NativePlatformEntrySetup profile={profile} />);
    expect(screen.getByLabelText("Native chat channel")).toHaveDisplayValue(
      "Saved help bot · Discord",
    );
    expect(screen.getByLabelText("Native chat channel")).toBeDisabled();
    expect(screen.getByLabelText("Native delivery transport")).toHaveValue(
      "direct",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Review current native entry" }),
    );
    expect(fixture.lifecycle).toHaveBeenCalledWith(
      "saved-channel",
      expect.any(Object),
    );
  } finally {
    fixture.bots.push(...savedBots);
  }
});

it("binds each retained-setup review label to its own checkbox", () => {
  render(
    <>
      <NativePlatformEntrySetup />
      <NativePlatformEntrySetup />
    </>,
  );
  for (const select of screen.getAllByLabelText("Native chat channel")) {
    fireEvent.change(select, { target: { value: fixture.bots[0]!.id } });
  }
  for (const select of screen.getAllByLabelText("Native delivery transport")) {
    fireEvent.change(select, { target: { value: "gateway" } });
  }
  for (const button of screen.getAllByRole("button", {
    name: "Inspect retained gateway setup",
  }))
    fireEvent.click(button);
  const review = {
    kind: "gateway",
    record_id: "record",
    attempt_id: "attempt",
    revision: 1,
    request_hash: `sha256:${"a".repeat(64)}`,
    credential_fingerprint: "b".repeat(64),
    remote_channel_id: null,
    remote_version: null,
    remote_ready: false,
  };
  act(() => {
    for (const call of fixture.inspect.mock.calls) call[1].onSuccess(review);
  });
  const checkboxes = screen.getAllByRole("checkbox", {
    name: "I reviewed this exact retained setup.",
  });
  expect(checkboxes[0]!.id).not.toBe(checkboxes[1]!.id);
  fireEvent.click(
    screen.getAllByText("I reviewed this exact retained setup.", {
      selector: "label",
    })[1]!,
  );
  expect(checkboxes[0]).not.toBeChecked();
  expect(checkboxes[1]).toBeChecked();
  const retries = screen.getAllByRole("button", {
    name: "Retry reviewed native setup",
  });
  expect(retries[0]).toBeDisabled();
  expect(retries[1]).toBeEnabled();
});
