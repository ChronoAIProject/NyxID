import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CallbackUrlCard } from "./callback-url-card";

const { mockPut } = vi.hoisted(() => ({
  mockPut: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    put: mockPut,
  },
  ApiError: class ApiError extends Error {
    status: number;
    errorCode: number;
    constructor(
      status: number,
      response: { message: string; error_code: number },
    ) {
      super(response.message);
      this.status = status;
      this.errorCode = response.error_code;
    }
  },
}));

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });

  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

function findButtonByIcon(className: string): HTMLElement | undefined {
  return screen
    .getAllByRole("button")
    .find((btn) => btn.querySelector(`.${className}`) !== null);
}

describe("CallbackUrlCard", () => {
  beforeEach(() => {
    mockPut.mockReset();
  });

  it("sends null callback_url when clearing input and clicking Save", async () => {
    const user = userEvent.setup();
    mockPut.mockResolvedValue({ id: "key-1", callback_url: null });

    render(
      <CallbackUrlCard
        keyId="key-1"
        callbackUrl="https://old.example.com/hook"
      />,
      { wrapper: createWrapper() },
    );

    const editButton = findButtonByIcon("lucide-pencil");
    if (!editButton) throw new Error("Edit button not found");
    await user.click(editButton);

    const input = screen.getByRole("textbox");
    await user.clear(input);
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(mockPut).toHaveBeenCalledWith("/api-keys/key-1", {
        callback_url: null,
      });
    });
  });

  it("sends null callback_url when clicking the clear button", async () => {
    const user = userEvent.setup();
    mockPut.mockResolvedValue({ id: "key-1", callback_url: null });

    render(
      <CallbackUrlCard
        keyId="key-1"
        callbackUrl="https://old.example.com/hook"
      />,
      { wrapper: createWrapper() },
    );

    const xButton = findButtonByIcon("lucide-x");
    if (!xButton) throw new Error("Clear button not found");
    await user.click(xButton);

    await waitFor(() => {
      expect(mockPut).toHaveBeenCalledWith("/api-keys/key-1", {
        callback_url: null,
      });
    });
  });

  it("shows 'Not set' when callbackUrl is null", () => {
    render(<CallbackUrlCard keyId="key-1" callbackUrl={null} />, {
      wrapper: createWrapper(),
    });

    expect(screen.getByText("Not set")).toBeInTheDocument();
  });

  it("does not show clear button when callbackUrl is null", () => {
    render(<CallbackUrlCard keyId="key-1" callbackUrl={null} />, {
      wrapper: createWrapper(),
    });

    expect(findButtonByIcon("lucide-x")).toBeUndefined();
  });

  it("resets input to current prop when re-entering edit mode", async () => {
    const user = userEvent.setup();
    const Wrapper = createWrapper();

    const { rerender } = render(
      <Wrapper>
        <CallbackUrlCard
          keyId="key-1"
          callbackUrl="https://old.example.com/hook"
        />
      </Wrapper>,
    );

    const editButton = findButtonByIcon("lucide-pencil");
    if (!editButton) throw new Error("Edit button not found");
    await user.click(editButton);

    const input = screen.getByRole("textbox") as HTMLInputElement;
    expect(input.value).toBe("https://old.example.com/hook");

    await user.click(screen.getByRole("button", { name: "Cancel" }));

    rerender(
      <Wrapper>
        <CallbackUrlCard keyId="key-1" callbackUrl={null} />
      </Wrapper>,
    );

    const editButton2 = findButtonByIcon("lucide-pencil");
    if (!editButton2) throw new Error("Edit button not found after rerender");
    await user.click(editButton2);

    const input2 = screen.getByRole("textbox") as HTMLInputElement;
    expect(input2.value).toBe("");
  });
});
