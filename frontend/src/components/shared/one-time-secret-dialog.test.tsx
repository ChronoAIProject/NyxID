import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { toast } from "sonner";
import { appI18n } from "@/i18n";
import { copyToClipboard } from "@/lib/utils";
import { OneTimeSecretDialog } from "./one-time-secret-dialog";

vi.mock("@/lib/utils", async (importOriginal) => ({
  ...await importOriginal<typeof import("@/lib/utils")>(),
  copyToClipboard: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

afterEach(async () => {
  cleanup();
  await appI18n.changeLanguage("en");
  vi.clearAllMocks();
});

it("uses the current language for field, copy action and delayed feedback", async () => {
  const value = "ExampleValue-001";
  let finishCopy!: () => void;
  vi.mocked(copyToClipboard).mockImplementationOnce(() => new Promise<void>((resolve) => {
    finishCopy = resolve;
  }));
  render(<OneTimeSecretDialog open onOpenChange={vi.fn()} title="Example" description="Example"
    values={[{ label: "Delivery signing secret", value }]} />);

  await userEvent.click(screen.getByRole("button", { name: "Copy Delivery signing secret" }));
  await act(() => appI18n.changeLanguage("zh-CN").then(() => undefined));
  expect(screen.getByText("投递签名密钥")).toBeInTheDocument();
  expect(screen.getByText(value)).toBeInTheDocument();
  finishCopy();
  await waitFor(() => expect(toast.success).toHaveBeenCalledWith("已复制投递签名密钥"));

  vi.mocked(copyToClipboard).mockRejectedValueOnce(new Error("unavailable"));
  await userEvent.click(screen.getByRole("button", { name: "复制 投递签名密钥" }));
  expect(toast.error).toHaveBeenCalledWith("复制失败 投递签名密钥");
  expect(copyToClipboard).toHaveBeenNthCalledWith(1, value);
  expect(copyToClipboard).toHaveBeenNthCalledWith(2, value);
});
