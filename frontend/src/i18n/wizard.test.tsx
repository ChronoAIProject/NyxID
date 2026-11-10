import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

vi.mock("@/i18n", () => {
  throw new Error("The standalone wizard must not import app catalogs");
});
vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({ data: [], isLoading: false }),
}));
vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({ data: [], isLoading: false }),
}));
vi.mock("@/hooks/use-nodes", () => ({
  useNodes: () => ({ data: [], isLoading: false }),
}));
const { post } = vi.hoisted(() => ({
  post: vi.fn().mockResolvedValue({ id: "key-id", full_key: "example-only" }),
}));
vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api-client")>()),
  api: { post },
}));
import { appI18n } from "./instance";
import { WizardShell } from "@/components/cli-wizard/shell";
import { ApiKeyCreateConfirm } from "@/components/cli-wizard/confirm-panels";
import { DisplayOncePanel } from "@/components/cli-wizard/display-once-panel";

afterEach(async () => {
  cleanup();
  await appI18n.changeLanguage("en");
  vi.clearAllMocks();
});

it("switches standalone wizard languages without losing form values or changing API scopes", async () => {
  await appI18n.changeLanguage("en");
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <WizardShell
        context="local"
        step={{ current: 2, total: 3, label: "configure scope" }}
      >
        <ApiKeyCreateConfirm
          prefill={{ name: "Original", scopes: "read write" }}
          pairingId="local"
          onSuccess={vi.fn()}
        />
      </WizardShell>
    </QueryClientProvider>,
  );
  fireEvent.change(screen.getByLabelText("Name"), {
    target: { value: "My draft 保留" },
  });
  const user = userEvent.setup();
  await user.click(screen.getByRole("combobox", { name: "Language" }));
  await user.click(screen.getByRole("option", { name: "简体中文" }));
  expect(screen.getByRole("heading", { name: "创建 API 密钥" })).toBeVisible();
  expect(screen.getByLabelText("名称")).toHaveValue("My draft 保留");
  expect(screen.getByText("第 2 步，共 3 步 · 配置权限范围")).toBeVisible();
  expect(screen.getByLabelText("read")).toBeChecked();
  expect(screen.getByLabelText("write")).toBeChecked();
  expect(post).not.toHaveBeenCalled();
  await act(async () => {
    await appI18n.changeLanguage("zh-TW");
  });
  expect(screen.getByRole("heading", { name: "建立 API 金鑰" })).toBeVisible();
  expect(screen.getByLabelText("名稱")).toHaveValue("My draft 保留");
  expect(appI18n.hasResourceBundle("en", "app")).toBe(false);
  expect(appI18n.hasResourceBundle("en", "translation")).toBe(false);
  expect(appI18n.hasResourceBundle("en", "onboarding")).toBe(false);
  client.clear();
});

it("keeps the same one-time secret masked when its screen changes language", async () => {
  await appI18n.changeLanguage("en");
  const onAcknowledge = vi.fn();
  render(
    <DisplayOncePanel
      title="API key created"
      description="Save this key now — it won't be shown again."
      secret="nyx_example_verbatim"
      ackButtonLabel="I have saved this — close"
      onAcknowledge={onAcknowledge}
      isAcknowledging={false}
    />,
  );
  expect(screen.queryByText("nyx_example_verbatim")).not.toBeInTheDocument();
  await act(async () => {
    await appI18n.changeLanguage("zh-TW");
  });
  expect(screen.getByText("僅顯示一次，請立即儲存")).toBeVisible();
  expect(screen.queryByText("nyx_example_verbatim")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "顯示" }));
  expect(screen.getByText("nyx_example_verbatim")).toBeVisible();
  expect(onAcknowledge).not.toHaveBeenCalled();
});
