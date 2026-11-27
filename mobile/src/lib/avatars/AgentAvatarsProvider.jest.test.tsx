import { act, render, screen } from "@testing-library/react-native";
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { Text } from "react-native";
import { AgentAvatarsProvider } from "./AgentAvatarsProvider";
import { useAgentAvatarsEnabled } from "./agentAvatarsFlag";

const mockProfile = jest.fn();
let mockSignedIn = true;
jest.mock("../api/http", () => ({
  getCurrentUserProfileRequest: (signal?: AbortSignal) => mockProfile(signal),
}));
jest.mock("../../features/auth/AuthSessionContext", () => ({
  useAuthSession: () => ({ isAuthenticated: mockSignedIn }),
}));

const profile = (features: string[]) => ({ capabilities: { enabled_features: features } });

function Flag() {
  return <Text>{useAgentAvatarsEnabled() ? "avatars on" : "avatars off"}</Text>;
}

/** The notification bell's observer of the same profile query. */
function Bell() {
  useQuery({
    queryKey: ["account", "profile"],
    queryFn: ({ signal }) => mockProfile(signal),
    staleTime: 60_000,
    retry: false,
  });
  return null;
}

async function mount(client: QueryClient) {
  await render(
    <QueryClientProvider client={client}>
      <AgentAvatarsProvider>
        <Flag />
        <Bell />
      </AgentAvatarsProvider>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  mockProfile.mockReset();
  mockSignedIn = true;
});

test("shares the start-up profile request with existing observers", async () => {
  mockProfile.mockResolvedValue(profile(["assistant:avatars"]));
  const client = new QueryClient({ defaultOptions: { queries: { gcTime: Infinity } } });
  await mount(client);
  expect(await screen.findByText("avatars on")).toBeTruthy();
  expect(mockProfile).toHaveBeenCalledTimes(1);
});

test("an admin toggle applies on the next foreground refetch", async () => {
  mockProfile.mockResolvedValue(profile(["assistant:avatars"]));
  const client = new QueryClient({ defaultOptions: { queries: { gcTime: Infinity } } });
  await mount(client);
  expect(await screen.findByText("avatars on")).toBeTruthy();
  mockProfile.mockResolvedValue(profile([]));
  // App.tsx refetches every active query when the app returns to the foreground.
  await act(async () => {
    await client.refetchQueries({ type: "active" });
  });
  expect(mockProfile).toHaveBeenCalledTimes(2);
  expect(await screen.findByText("avatars off")).toBeTruthy();
});

test("signed out: no request and today's avatars", async () => {
  mockSignedIn = false;
  const client = new QueryClient({ defaultOptions: { queries: { gcTime: Infinity } } });
  await render(
    <QueryClientProvider client={client}>
      <AgentAvatarsProvider>
        <Flag />
      </AgentAvatarsProvider>
    </QueryClientProvider>,
  );
  expect(screen.getByText("avatars off")).toBeTruthy();
  expect(mockProfile).not.toHaveBeenCalled();
});
