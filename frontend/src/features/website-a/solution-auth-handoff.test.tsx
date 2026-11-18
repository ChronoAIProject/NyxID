import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import {
  createMemoryHistory,
  createRootRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { router as appRouter } from "@/router";
import { LoginPage } from "@/pages/login";
import { RegisterPage } from "@/pages/register";
import { useAuthStore } from "@/stores/auth-store";
import { resolveTrustedAuthReturnTo } from "@/lib/return-url";
import { SolutionActivation } from "./solution-activation";
import { siteHref, solutionAuthPath, legacyDestination } from "./auth-links";
import { websiteRelease } from "./release-config";
import { WebsiteContext, type WebsiteState } from "./website-context";

const { login, register } = vi.hoisted(() => ({
  login: vi.fn(),
  register: vi.fn(),
}));
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({
    data: {
      invite_code_required: true,
      email_auth_enabled: true,
      social_providers: [],
    },
  }),
}));
vi.mock("@/hooks/use-auth", () => ({
  useLogin: () => ({ mutateAsync: login, isPending: false }),
  useRegister: () => ({ mutateAsync: register, isPending: false }),
}));
vi.mock("@/hooks/use-auth-device", () => ({
  useWebAuthDeviceLogin: () => ({
    phase: "idle",
    request: null,
    remainingSeconds: null,
    start: vi.fn(),
    close: vi.fn(),
  }),
}));

const originalAuth = useAuthStore.getState();
const originalSolutions = { ...websiteRelease.solutions };
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.clearAllMocks();
  useAuthStore.setState(originalAuth);
  websiteRelease.solutions = { ...originalSolutions };
  window.history.replaceState({}, "", "/");
});

function runRealStartHandler() {
  const route = Object.values(appRouter.routesById).find(
    (r) => r.fullPath === "/start",
  )!;
  const beforeLoad = route.options.beforeLoad as () => void;
  beforeLoad();
}

it.each([
  "/assistant?setup=help-desk&draft=hello%20world",
  "/dashboard?panel=settings",
])(
  "uses the actual /start route and shared auth handoff for %s",
  async (target) => {
    websiteRelease.solutions = { "help-desk": target };
    const search =
      "?t=help-desk&code=NYX-ABCDEF12&return_to=https%3A%2F%2Fevil.test";
    window.history.replaceState({}, "", `/start${search}`);
    const replace = vi
      .spyOn(window.location, "replace")
      .mockImplementation(() => {});
    runRealStartHandler();
    const href = String(replace.mock.calls[0]?.[0]);
    expect(href).toBe(siteHref(solutionAuthPath(target)));
    expect(href).toBe(legacyDestination("#/start?t=help-desk", search));
    const destination = new URL(href, window.location.origin);
    expect(destination.pathname).toBe("/login");
    expect(destination.searchParams.get("code")).toBe("NYX-ABCDEF12");
    expect(destination.searchParams.get("return_to")).toBe(target);
    window.history.replaceState({}, "", href);
    await showAuthPage(LoginPage);
    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "Sign up" }));
    await waitFor(() => expect(window.location.pathname).toBe("/register"));
    expect(screen.getByPlaceholderText("NYX-XXXXXXXX")).toHaveValue(
      "NYX-ABCDEF12",
    );
    expect(new URLSearchParams(window.location.search).get("return_to")).toBe(
      target,
    );
  },
);

it.each([
  ["?t=help-desk&code=NYX-ABCDEF12", "/solutions/help-desk"],
  ["?code=NYX-ABCDEF12&return_to=%2Fkeys", "/register"],
])("preserves the actual /start fallback for %s", (search, path) => {
  websiteRelease.solutions = {};
  window.history.replaceState({}, "", `/start${search}`);
  const replace = vi
    .spyOn(window.location, "replace")
    .mockImplementation(() => {});
  runRealStartHandler();
  expect(replace).toHaveBeenCalledWith(`${path}${search}`);
});

async function showAuthPage(component: typeof LoginPage) {
  const router = createRouter({
    routeTree: createRootRoute({ component }),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  await router.load();
  return render(<RouterProvider router={router} />);
}

it.each(["/solutions/help-desk", "/zh/solutions/help-desk"])(
  "retains the real solution target and top-level invitation through login and registration from %s",
  async (path) => {
    const user = userEvent.setup();
    const target = "/assistant?setup=help-desk&draft=hello%20world";
    window.history.replaceState(
      {},
      "",
      `${path}?code=NYX-ABCDEF12&return_to=https%3A%2F%2Fevil.test&utm_source=campaign`,
    );
    const view = render(
      <WebsiteContext.Provider
        value={
          {
            language: path.startsWith("/zh") ? "zh" : "en",
            route: "/solutions/help-desk",
            siteHref,
          } as WebsiteState
        }
      >
        <SolutionActivation targets={{ "help-desk": target }} />
      </WebsiteContext.Provider>,
    );
    const href = screen.getByRole("link").getAttribute("href")!;
    const destination = new URL(href, window.location.origin);
    expect(destination.pathname).toBe("/login");
    expect(destination.searchParams.get("code")).toBe("NYX-ABCDEF12");
    expect(destination.searchParams.get("return_to")).toBe(target);
    expect(destination.searchParams.get("utm_source")).toBe("campaign");
    expect(
      resolveTrustedAuthReturnTo(destination.searchParams.get("return_to")!),
    ).toBe(new URL(target, window.location.origin).href);
    view.unmount();
    window.history.replaceState({}, "", href);

    // Exercise the real app route validator and signed-out auth-layout gate.
    const route = Object.values(appRouter.routesById).find(
      (r) => r.fullPath === "/login",
    )!;
    const validate = route.options.validateSearch as (
      s: Record<string, string>,
    ) => Record<string, string>;
    expect(validate(Object.fromEntries(destination.searchParams))).toEqual({
      return_to: target,
      code: "NYX-ABCDEF12",
    });
    useAuthStore.setState({ isAuthenticated: false, isLoading: false });
    const gate = route.parentRoute!.options.beforeLoad as () => unknown;
    expect(gate()).toBeUndefined();
    const loginPage = await showAuthPage(LoginPage);
    await user.click(screen.getByRole("button", { name: "Sign up" }));
    await waitFor(() => expect(window.location.pathname).toBe("/register"));
    expect(screen.getByPlaceholderText("NYX-XXXXXXXX")).toHaveValue(
      "NYX-ABCDEF12",
    );
    expect(new URLSearchParams(window.location.search).get("return_to")).toBe(
      target,
    );
    loginPage.unmount();
    await showAuthPage(RegisterPage);
    expect(screen.getByPlaceholderText("NYX-XXXXXXXX")).toHaveValue(
      "NYX-ABCDEF12",
    );
    await user.click(
      screen.getByRole("button", { name: /Continue with Email/i }),
    );
    const name = screen.getByPlaceholderText("John Doe");
    await waitFor(() => expect(name).toHaveFocus());
    await user.type(name, "Ada Lovelace");
    await user.type(
      screen.getByPlaceholderText("you@example.com"),
      "ada@example.com",
    );
    await user.type(
      screen.getByPlaceholderText("Min 8 characters"),
      "Hunter22",
    );
    await user.type(
      screen.getByPlaceholderText("Re-enter your password"),
      "Hunter22",
    );
    register.mockResolvedValue({ message: "Check your email." });
    await user.click(screen.getByRole("button", { name: "Create Account" }));
    await waitFor(() =>
      expect(register).toHaveBeenCalledWith(
        expect.objectContaining({ invite_code: "NYX-ABCDEF12" }),
      ),
    );
    await waitFor(() => expect(window.location.pathname).toBe("/login"));
    expect(new URLSearchParams(window.location.search).get("return_to")).toBe(
      target,
    );
    login.mockResolvedValue({ mfaRequired: false });
    const assign = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    await user.type(
      screen.getByPlaceholderText("you@example.com"),
      "ada@example.com",
    );
    await user.type(
      screen.getByPlaceholderText("Enter your password"),
      "Hunter22",
    );
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    await waitFor(() =>
      expect(assign).toHaveBeenCalledWith(
        new URL(target, window.location.origin).href,
      ),
    );
    expect(assign).not.toHaveBeenCalledWith("https://evil.test/");
  },
);

it("keeps the authenticated auth-layout handoff on the trusted configured target", () => {
  const target = "/dashboard?panel=settings";
  window.history.replaceState(
    {},
    "",
    `/login?return_to=${encodeURIComponent(target)}&code=NYX-ABCDEF12`,
  );
  useAuthStore.setState({ isAuthenticated: true, isLoading: false });
  const assign = vi
    .spyOn(window.location, "assign")
    .mockImplementation(() => {});
  const route = Object.values(appRouter.routesById).find(
    (r) => r.fullPath === "/login",
  )!;
  const gate = route.parentRoute!.options.beforeLoad as () => unknown;
  act(() => {
    gate();
  });
  expect(assign).toHaveBeenCalledWith(
    new URL(target, window.location.origin).href,
  );
});
