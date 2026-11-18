import React from "react";
import { act, fireEvent, render } from "@testing-library/react-native";
import { AppState, Keyboard, Platform } from "react-native";
import { AuthHomeScreen } from "./AuthHomeScreen";
import { spacing } from "../../theme/designTokens";
import { mobileApi } from "../../lib/api/mobileApi";
import { ApiError } from "../../lib/api/ApiError";
import { MfaChallengeError } from "../../lib/auth/mfaChallenge";

const mockScrollTo = jest.fn();
const mockViewport = {};
const mockSignIn = jest.fn();
jest.mock("../../lib/api/mobileApi", () => ({
  mobileApi: { loginWithPassword: jest.fn(), verifyMfa: jest.fn() },
}));
jest.mock("./AuthSessionContext", () => ({
  useAuthSession: () => ({ signInWithSession: mockSignIn }),
}));
jest.mock("../../lib/auth/sessionStore", () => ({ authSessionGeneration: () => 0 }));
jest.mock("../../lib/telemetry", () => ({ capture: jest.fn() }));
jest.mock("../../lib/api/publicAuthApi", () => ({ getPublicAuthConfig: jest.fn() }));
jest.mock("@react-navigation/native", () => ({ useIsFocused: () => true }));
jest.mock("@tanstack/react-query", () => ({
  useQuery: () => ({
    data: { emailAuthEnabled: true, socialProviders: ["google", "github", "apple"] },
    isPending: false, isError: false,
  }),
}));
jest.mock("expo-web-browser", () => ({}));
jest.mock("../../lib/env", () => ({ getFrontendUrl: () => "https://nyxid.ai" }));
jest.mock("../../i18n/LanguageProvider", () => ({
  useAppLanguage: () => ({ language: "en" }),
}));
jest.mock("../../i18n/Translation", () => ({
  Translation: ({ message }: { message: string }) => {
    const { Text } = require("react-native");
    return <Text>{message}</Text>;
  },
}));
jest.mock("./useAuthTheme", () => ({
  useAuthTheme: () => ({
    colors: require("../../theme/mobileTheme").darkColors,
    mode: "dark",
  }),
}));
jest.mock("../../components/ScreenContainer", () => ({
  ScreenContainer: require("react-native").View,
}));
jest.mock("../../components/ToastOverlay", () => ({ ToastOverlay: () => null }));
jest.mock("../../components/brand/BrandNebula", () => ({
  BrandNebula: () => null,
  BreathingMark: require("react-native").View,
  RiseIn: require("react-native").View,
}));
jest.mock("../../components/brand/PortalMark", () => ({ PortalMarkLogo: () => null }));
jest.mock("../../components/workspace/KeyboardScrollView", () => ({
  KeyboardScrollView: ({ scrollRef, ...props }: any) => {
    const { ScrollView } = require("react-native");
    require("react").useImperativeHandle(scrollRef, () => ({
      getNativeScrollRef: () => mockViewport,
      scrollTo: mockScrollTo,
    }));
    return <ScrollView {...props} />;
  },
}));

beforeEach(() => {
  jest.clearAllMocks();
  jest.spyOn(Keyboard, "dismiss").mockImplementation(() => {});
  jest.spyOn(AppState, "addEventListener").mockReturnValue({ remove: jest.fn() });
});
afterEach(() => { jest.restoreAllMocks(); });

const invalidCredentials = () => new ApiError({
  errorKey: "authentication_failed", errorCode: 2000, statusCode: 401,
  message: "Account lookup failed for the supplied address",
});

for (const platform of ["ios", "android"] as const) {
  test(`${platform}: a rejected password request reveals a persistent, generic error and permits retry`, async () => {
    jest.replaceProperty(Platform, "OS", platform);
    jest.mocked(mobileApi.loginWithPassword).mockRejectedValue(invalidCredentials());
    const screen = await render(<AuthHomeScreen navigation={{} as never} route={{} as never} />);
    await fireEvent.changeText(screen.getByLabelText("Email"), "unknown@example.test");
    await fireEvent.changeText(screen.getByLabelText("Password"), "wrong-password");
    await fireEvent.press(screen.getByRole("button", { name: "Sign In" }));

    expect(screen.getByRole("button", { name: "Sign In" }).props.accessibilityState.busy).toBe(false);
    const error = screen.getByRole("alert");
    expect(screen.getByText("Invalid email or password.")).toBeTruthy();
    expect(screen.queryByText(/Account lookup failed/)).toBeNull();
    expect(error.props.accessibilityLiveRegion).toBe("assertive");
    const measureLayout = jest.fn((_viewport, measured) => measured(0, 620));
    await fireEvent(error, "layout", { currentTarget: { measureLayout } });
    expect(Keyboard.dismiss).toHaveBeenCalledTimes(1);
    expect(measureLayout).toHaveBeenCalledWith(mockViewport, expect.any(Function));
    expect(mockScrollTo).toHaveBeenCalledWith({ y: 620 - spacing.lg, animated: false });
    expect(mockSignIn).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Email").props.value).toBe("unknown@example.test");
    expect(screen.getByLabelText("Password").props.value).toBe("wrong-password");

    // A rerender preserves the alert. Resubmitting clears it until the response arrives.
    await screen.rerender(<AuthHomeScreen navigation={{} as never} route={{} as never} />);
    expect(screen.getByText("Invalid email or password.")).toBeTruthy();
    let failRetry!: (error: ApiError) => void;
    jest.mocked(mobileApi.loginWithPassword).mockImplementationOnce(
      () => new Promise((_resolve, reject) => { failRetry = reject; }),
    );
    await fireEvent.press(screen.getByRole("button", { name: "Sign In" }));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByRole("button", { name: "Signing in..." }).props.accessibilityState.busy).toBe(true);
    await act(async () => { failRetry(invalidCredentials()); });
    expect(mobileApi.loginWithPassword).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("alert")).toBeTruthy();
    const field = platform === "ios" ? "Email" : "Password";
    await fireEvent.changeText(screen.getByLabelText(field), field === "Email" ? "member@example.test" : "try-again");
    expect(screen.queryByRole("alert")).toBeNull();
    await screen.unmount();
  });

  test(`${platform}: MFA verification errors use the same visible retry feedback`, async () => {
    jest.replaceProperty(Platform, "OS", platform);
    jest.mocked(mobileApi.loginWithPassword).mockRejectedValue(new MfaChallengeError("test-mfa-challenge"));
    jest.mocked(mobileApi.verifyMfa).mockRejectedValue(new ApiError({
      errorKey: "bad_request", errorCode: 1000, statusCode: 400,
      message: "Invalid verification code.",
    }));
    const screen = await render(<AuthHomeScreen navigation={{} as never} route={{} as never} />);
    await fireEvent.changeText(screen.getByLabelText("Email"), "member@example.test");
    await fireEvent.changeText(screen.getByLabelText("Password"), "password");
    await fireEvent.press(screen.getByRole("button", { name: "Sign In" }));
    expect(screen.queryByRole("alert")).toBeNull();
    await fireEvent.changeText(screen.getByLabelText("Authenticator code"), "012345");
    await fireEvent.press(screen.getByRole("button", { name: "Verify & sign in" }));
    const error = screen.getByRole("alert");
    expect(screen.getByText("Invalid verification code.")).toBeTruthy();
    expect(error.props.accessibilityLiveRegion).toBe("assertive");
    expect(screen.getByLabelText("Authenticator code").props.value).toBe("");
    await fireEvent.changeText(screen.getByLabelText("Authenticator code"), "012346");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(mockSignIn).not.toHaveBeenCalled();
    await screen.unmount();
  });
}
