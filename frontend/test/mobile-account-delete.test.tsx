import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { runInNewContext } from "node:vm";
import * as React from "react";
import * as jsxRuntime from "react/jsx-runtime";
import * as query from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import ts from "typescript";
import { describe, expect, it, vi } from "vitest";

type NativeButton = { text: string; style?: string; onPress?: () => void };
type NativeProps = { children?: React.ReactNode; disabled?: boolean; onPress?: () => void };
const mobileRoot = resolve("../mobile/src");

/** Render current mobile source with native primitives shimmed into happy-dom.
 * The account screen, PrimaryButton, mobileApi and HTTP request helper are real.
 * Native Alert is captured at the OS boundary; no confirmation copy is mocked.
 */
function harness() {
  const alerts: { title: string; message: string; buttons: NativeButton[] }[] = [];
  const transport = vi.fn<typeof fetch>().mockResolvedValue(new Response(JSON.stringify({
    error: "validation_error", error_code: 1000, message: "Fixture denial after request capture",
  }), { status: 400, headers: { "Content-Type": "application/json" } }));
  const cache = new Map<string, Record<string, unknown>>();
  const box = ({ children }: NativeProps) => <div>{children}</div>;
  const native = {
    Alert: { alert: (title: string, message: string, buttons: NativeButton[]) => alerts.push({ title, message, buttons }) },
    Image: () => null,
    Linking: { openSettings: vi.fn() },
    Pressable: ({ children, disabled, onPress }: NativeProps) => <button disabled={disabled} onClick={onPress}>{children}</button>,
    ScrollView: box,
    SafeAreaView: box,
    StyleSheet: { create: (styles: unknown) => styles },
    Switch: () => <input type="checkbox" readOnly />,
    Text: ({ children }: NativeProps) => <span>{children}</span>,
    View: box,
  };

  function load(path: string): Record<string, unknown> {
    const cached = cache.get(path);
    if (cached) return cached;
    const code = ts.transpileModule(readFileSync(path, "utf8"), {
      compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX, esModuleInterop: true },
    }).outputText;
    const exported: Record<string, unknown> = {};
    cache.set(path, exported);
    runInNewContext(code, {
      exports: exported,
      process: { env: { EXPO_PUBLIC_API_BASE_URL: "https://mobile.example.test/api/v1" } },
      fetch: transport,
      __DEV__: false,
      setTimeout, clearTimeout, URL,
      require: (name: string): unknown => {
        if (name === "react") return React;
        if (name === "react/jsx-runtime") return jsxRuntime;
        if (name === "@tanstack/react-query") return query;
        if (name === "react-native") return native;
        if (name === "react-native-svg") return { __esModule: true, default: box, Circle: () => null, Path: () => null };
        if (name === "expo-linear-gradient") return { LinearGradient: box };
        if (name === "expo-constants") return { __esModule: true, default: { expoConfig: { version: "test" } } };
        if (name.endsWith("/ThemeContext")) return { useTheme: () => ({ colors: load(resolve(mobileRoot, "theme/mobileTheme.ts")).darkColors, mode: "dark", preference: "dark", setPreference: vi.fn() }) };
        if (name.endsWith("/AuthSessionContext")) return { useAuthSession: () => ({ signOut: vi.fn() }) };
        if (name.endsWith("/useNetworkStatus")) return { useNetworkStatus: () => ({ isConnected: true, recheckConnection: vi.fn() }) };
        if (name.endsWith("/sessionStore")) return { loadStoredAuthSession: async () => ({ accessToken: "fixture-token" }), clearStoredAuthSession: vi.fn(), persistAuthSession: vi.fn() };
        if (name.endsWith("/telemetry")) return { capture: vi.fn() };
        if (name.endsWith("/pushNotifications")) return { activatePushAfterLogin: vi.fn() };
        if (name.endsWith("/errorMessages")) return { resolveErrorMessage: vi.fn() };
        if (name.endsWith("/OfflineBanner")) return { OfflineBanner: () => null };
        if (name.endsWith("/TelegramLinkModal")) return { TelegramLinkModal: () => null };
        if (name.endsWith("/ToastOverlay")) return { ToastOverlay: () => null };
        if (name === "./authDeviceApi") return {};
        if (name.startsWith(".")) {
          const target = resolve(dirname(path), name);
          try { return load(`${target}.ts`); }
          catch (error) { if ((error as { code?: string }).code !== "ENOENT") throw error; }
          return load(`${target}.tsx`);
        }
        throw new Error(`Unexpected mobile test dependency: ${name}`);
      },
    }, { filename: path });
    return exported;
  }

  const AccountSettingsScreen = load(resolve(mobileRoot, "features/account/AccountSettingsScreen.tsx")).AccountSettingsScreen as React.ComponentType<{ navigation: { navigate: () => void } }>;
  const client = new query.QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity }, mutations: { retry: false } } });
  client.setQueryData(["account", "profile"], { id: "owner", display_name: "Owner", email: "owner@example.test" });
  client.setQueryData(["account", "notificationSettings"], { push_device_count: 1, push_enabled: true, telegram_connected: false });
  render(<query.QueryClientProvider client={client}><AccountSettingsScreen navigation={{ navigate: vi.fn() }} /></query.QueryClientProvider>);
  return { alerts, transport, client };
}

describe("actual mobile account deletion confirmation", () => {
  it("discloses every managed Machine, paused workspace and retained record before the native confirmation", async () => {
    const { alerts, transport, client } = harness();
    await userEvent.click(screen.getByRole("button", { name: "Delete Account" }));
    expect(alerts).toHaveLength(1);
    const message = alerts[0]!.message;
    expect(message).toMatch(/all (?:of your )?managed Machines/i);
    expect(message).toMatch(/paused \/workspace disks/i);
    expect(message).toMatch(/temporary (?:files|data)/i);
    expect(message).toMatch(/irreversible/i);
    expect(message).toMatch(/no snapshots/i);
    expect(message).toMatch(/no recovery or restore/i);
    expect(message).toMatch(/24 hours/i);
    expect(message).toMatch(/cleanup pending/i);
    expect(message).toMatch(/at most 1 hour/i);
    expect(message).toMatch(/30 days/i);
    expect(message).toMatch(/90 days/i);
    expect(message).toMatch(/audit and financial records/i);
    expect(transport).not.toHaveBeenCalled();
    client.clear();
  });

  it("keeps cancellation free of account deletion effects", async () => {
    const { alerts, transport, client } = harness();
    await userEvent.click(screen.getByRole("button", { name: "Delete Account" }));
    const cancel = alerts[0]!.buttons.find((button) => button.style === "cancel");
    expect(cancel).toBeDefined();
    await act(async () => cancel?.onPress?.());
    expect(transport).not.toHaveBeenCalled();
    client.clear();
  });

  it("sends the accepted policy through the real mobile HTTP helper only on explicit native confirmation", async () => {
    const { alerts, transport, client } = harness();
    await userEvent.click(screen.getByRole("button", { name: "Delete Account" }));
    expect(transport).not.toHaveBeenCalled();
    await act(async () => alerts[0]!.buttons.find((button) => button.style === "destructive")!.onPress!());
    await waitFor(() => expect(transport).toHaveBeenCalledWith("https://mobile.example.test/api/v1/users/me", expect.objectContaining({
      method: "DELETE",
      headers: expect.objectContaining({ "X-NyxID-Account-Deletion-Policy": "machine-data" }),
    })));
    client.clear();
  });
});
