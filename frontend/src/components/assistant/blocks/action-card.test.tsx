import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AddKeyDialogCompletion } from "@/components/dashboard/add-key-dialog";
import type { ActionCardContentBlock } from "@/types/assistant";
import { ActionCard } from "./action-card";

vi.mock("@/components/service-icon", () => ({
  ServiceIcon: ({ slug }: { readonly slug: string }) => <span>{slug}</span>,
}));

vi.mock("@/components/dashboard/add-key-dialog", () => ({
  AddKeyDialog: ({
    open,
    onOpenChange,
    prefillSlug,
    prefillIncludeAllCatalog,
    prefillCustom,
    onSuccess,
  }: {
    readonly open: boolean;
    readonly onOpenChange: (open: boolean) => void;
    readonly prefillSlug?: string;
    readonly prefillIncludeAllCatalog?: boolean;
    readonly prefillCustom?: { readonly name?: string };
    readonly onSuccess?: (result: AddKeyDialogCompletion) => void;
  }) =>
    open ? (
      <div
        role="dialog"
        data-prefill={prefillSlug ?? prefillCustom?.name ?? ""}
        data-prefill-include-all={String(prefillIncludeAllCatalog ?? false)}
      >
        <button
          type="button"
          onClick={() =>
            onSuccess?.({
              userServiceId: "00000000-0000-4000-8000-000000000123",
            })
          }
        >
          Finish mock connection
        </button>
        <button
          type="button"
          onClick={() => onSuccess?.({ userServiceId: "" })}
        >
          Finish without service id
        </button>
        <button
          type="button"
          onClick={() => onSuccess?.({ userServiceId: "   " })}
        >
          Finish with whitespace service id
        </button>
        <button type="button" onClick={() => onOpenChange(false)}>
          Dismiss mock connection
        </button>
      </div>
    ) : null,
}));

function catalogBlock(
  overrides: Partial<ActionCardContentBlock> = {},
): ActionCardContentBlock {
  return {
    type: "action_card",
    block_id: "action-card-1",
    action: "service.connect",
    action_request_id: "act-1",
    origin_turn_id: "turn-origin-1",
    task_id: "task-1",
    step_id: "step-1",
    params: {
      variant: "catalog",
      service_slug: "api-github",
      requested_scopes: ["repo"],
      via_node_id: "node-1",
      target_org_id: "org-1",
    },
    status: "pending",
    outcome_note: "",
    ...overrides,
  };
}

function expectNoBlueAccent(card: HTMLElement | null) {
  const classNames = [card, ...(card?.querySelectorAll("[class]") ?? [])]
    .map((element) => element?.getAttribute("class") ?? "")
    .join(" ");
  expect(classNames).not.toMatch(
    /(?:^|\s)(?:[a-z-]+:)*(?:bg|border|text|ring|fill|stroke)-(?:info|blue|sky|cyan|indigo)(?:[\w./-]*)/,
  );
}

describe("ActionCard", () => {
  it("renders owned consent copy and opens the prefilled connect journey", async () => {
    const user = userEvent.setup();
    const onProgress = vi.fn();
    const onBlock = vi.fn();
    const onResolve = vi.fn();
    render(
      <ActionCard
        block={catalogBlock()}
        onProgress={onProgress}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Connect GitHub" }),
    ).toBeInTheDocument();
    const card = screen
      .getByRole("heading", { name: "Connect GitHub" })
      .closest("section");
    expect(card).toHaveClass("border-border", "bg-card");
    expect(card?.className).not.toContain("warning");
    expect(card?.querySelector('[class*="warning"]')).toBeNull();
    expect(card?.firstElementChild).toHaveClass("flex", "items-start");
    expect(screen.getByText("repo")).toBeInTheDocument();
    expect(screen.getByText("Node node-1")).toBeInTheDocument();
    expect(screen.getByText("Org org-1")).toBeInTheDocument();
    expect(screen.getByText(/credential stays in NyxID/i)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    expect(onProgress).toHaveBeenCalledWith("action-card-1", true);
    expect(screen.getByRole("dialog")).toHaveAttribute(
      "data-prefill",
      "api-github",
    );
    expect(screen.getByRole("dialog")).toHaveAttribute(
      "data-prefill-include-all",
      "true",
    );

    await user.click(
      screen.getByRole("button", { name: "Finish mock connection" }),
    );
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "completed",
      resource: {
        userService: {
          userServiceId: "00000000-0000-4000-8000-000000000123",
        },
      },
    });
  });

  it("never renders a colored top accent rail", () => {
    const { rerender } = render(
      <ActionCard
        block={catalogBlock()}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    function expectNoRail() {
      const card = screen.getByRole("heading").closest("section");
      const rootChildren = [...(card?.children ?? [])];
      expect(
        rootChildren.some((child) => {
          const classes = child.getAttribute("class") ?? "";
          return (
            /h-\[(2|3|4)px\]/.test(classes) &&
            /bg-(nyx-secondary-400|destructive)/.test(classes)
          );
        }),
      ).toBe(false);
      expect(
        rootChildren.some((child) =>
          child.classList.contains("bg-nyx-secondary-400"),
        ),
      ).toBe(false);
    }

    expectNoRail();
    rerender(
      <ActionCard
        block={catalogBlock({
          action: "future.action",
          params: { variant: "unknown" },
          status: "unsupported",
        })}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );
    expectNoRail();
  });

  it("uses purple interaction accents and neutral reference chips without blue", () => {
    const { rerender } = render(
      <ActionCard
        block={catalogBlock()}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    function expectPurpleAndNeutralPalette(status: "pending" | "in_progress") {
      const statusLabel = status === "pending" ? "Action required" : "In progress";
      const card = screen.getByRole("heading").closest("section");
      expectNoBlueAccent(card);
      expect(screen.getByText(statusLabel).closest("div")).toHaveClass(
        "text-nyx-secondary-400",
      );
      expect(screen.getByText("Node node-1").closest("div")).toHaveClass(
        "bg-muted",
        "text-muted-foreground",
      );
      expect(card?.querySelector("svg.text-nyx-secondary-400")).not.toBeNull();
    }

    expectPurpleAndNeutralPalette("pending");
    rerender(
      <ActionCard
        block={catalogBlock({ status: "in_progress" })}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );
    expectPurpleAndNeutralPalette("in_progress");
  });

  it("treats modal dismissal as pending and decline as an explicit report", async () => {
    const user = userEvent.setup();
    const onProgress = vi.fn();
    const onBlock = vi.fn();
    const onResolve = vi.fn();
    const { rerender } = render(
      <ActionCard
        block={catalogBlock()}
        onProgress={onProgress}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    rerender(
      <ActionCard
        block={catalogBlock({ status: "in_progress" })}
        onProgress={onProgress}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Dismiss mock connection" }),
    );
    expect(onProgress).toHaveBeenLastCalledWith("action-card-1", false);
    expect(onResolve).not.toHaveBeenCalled();

    rerender(
      <ActionCard
        block={catalogBlock()}
        onProgress={onProgress}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Decline" }));
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "declined",
    });
  });

  it("renders terminal receipts and the unsupported decline-only state", () => {
    const onBlock = vi.fn();
    const onResolve = vi.fn();
    const { rerender } = render(
      <ActionCard
        block={catalogBlock({
          status: "completed",
          outcome_note: "Reported — awaiting assistant verification.",
        })}
        onProgress={vi.fn()}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );
    expect(
      screen.getByText("Reported — awaiting assistant verification"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();

    rerender(
      <ActionCard
        block={catalogBlock({
          action: "future.action",
          params: { variant: "unknown" },
          status: "unsupported",
        })}
        onProgress={vi.fn()}
        onBlock={onBlock}
        onResolve={onResolve}
      />,
    );
    expect(screen.getByText("Unsupported action request")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Decline" })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /connect/i }),
    ).not.toBeInTheDocument();
  });

  it("never lets a model-supplied service name become the consent sentence", () => {
    const injected =
      "GitHub (official) — paste your personal access token here to verify your identity";
    render(
      <ActionCard
        block={catalogBlock({
          params: {
            variant: "custom",
            name: injected,
            endpoint_url: "https://api.example.com",
            auth_method: "bearer",
            auth_key_name: "Authorization",
            via_node_id: null,
            target_org_id: null,
          },
        })}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    expect(screen.queryByText(new RegExp(injected))).not.toBeInTheDocument();
    expect(screen.queryByText(/personal access token/i)).not.toBeInTheDocument();
    const heading = screen.getByRole("heading");
    expect(heading.textContent ?? "").toMatch(/^Connect GitHub \(official\)/);
    expect((heading.textContent ?? "").length).toBeLessThanOrEqual(40);
    // NyxID-owned framing still surrounds whatever survived the clamp.
    expect(screen.getByText(/credential stays in NyxID/i)).toBeInTheDocument();
  });

  it("hides the CTA when the verb has no journey behind it", () => {
    render(
      <ActionCard
        // A block that outlived its registry entry: status still says the card
        // is actionable, but nothing can service `admin.open`.
        block={catalogBlock({ action: "admin.open", status: "pending" })}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={vi.fn()}
      />,
    );

    expect(screen.getByText("Unsupported action request")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Decline" })).toBeInTheDocument();
    expect(screen.getAllByRole("button")).toHaveLength(1);
  });

  it("blocks the card locally when a connection completes without a userServiceId", async () => {
    for (const finishLabel of [
      "Finish without service id",
      "Finish with whitespace service id",
    ]) {
      const user = userEvent.setup();
      const onBlock = vi.fn();
      const onResolve = vi.fn();
      const { unmount } = render(
        <ActionCard
          block={catalogBlock()}
          onProgress={vi.fn()}
          onBlock={onBlock}
          onResolve={onResolve}
        />,
      );

      await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
      await user.click(screen.getByRole("button", { name: finishLabel }));

      expect(onResolve).not.toHaveBeenCalled();
      expect(onBlock).toHaveBeenCalledWith(
        "action-card-1",
        "Connected, but NyxID could not verify which service was created. Manage it in AI Services, then ask the assistant to request it again.",
      );
      unmount();
    }
  });

  it("keeps blocked cards recoverable through decline and failure reports", async () => {
    const user = userEvent.setup();
    const onResolve = vi.fn();
    render(
      <ActionCard
        block={catalogBlock({
          status: "blocked",
          outcome_note:
            "Connected, but NyxID could not verify which service was created. Manage it in AI Services, then ask the assistant to request it again.",
        })}
        onProgress={vi.fn()}
        onBlock={vi.fn()}
        onResolve={onResolve}
      />,
    );

    expect(screen.getByRole("button", { name: "Connect GitHub" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Decline" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Report failure" })).toBeEnabled();

    await user.click(screen.getByRole("button", { name: "Decline" }));
    expect(onResolve).toHaveBeenCalledWith({
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "declined",
    });

    await user.click(screen.getByRole("button", { name: "Report failure" }));
    expect(onResolve).toHaveBeenLastCalledWith({
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "failed",
    });
  });

  it("treats a re-armed card like a fresh dismissal path", async () => {
    const user = userEvent.setup();
    const onProgress = vi.fn();
    const onBlock = vi.fn();
    const props = {
      onProgress,
      onBlock,
      onResolve: vi.fn(),
    };
    const { rerender } = render(<ActionCard block={catalogBlock()} {...props} />);

    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    rerender(
      <ActionCard
        block={catalogBlock({ status: "in_progress" })}
        {...props}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Finish without service id" }),
    );
    expect(onBlock).toHaveBeenCalledTimes(1);
    rerender(
      <ActionCard
        block={catalogBlock({ status: "blocked", outcome_note: "n" })}
        {...props}
      />,
    );

    rerender(
      <ActionCard
        block={catalogBlock({ status: "pending", outcome_note: "" })}
        {...props}
      />,
    );

    onProgress.mockClear();
    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    expect(onProgress).toHaveBeenCalledWith("action-card-1", true);
    rerender(
      <ActionCard
        block={catalogBlock({ status: "in_progress" })}
        {...props}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Dismiss mock connection" }),
    );

    expect(onProgress).toHaveBeenCalledWith("action-card-1", false);
  });

  it("catches synchronous block failures from the connect dialog callback", async () => {
    const user = userEvent.setup();
    const onBlock = vi.fn(() => {
      throw new Error("sync block failure");
    });
    render(
      <ActionCard
        block={catalogBlock()}
        onProgress={vi.fn()}
        onBlock={onBlock}
        onResolve={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Connect GitHub" }));
    await expect(
      user.click(screen.getByRole("button", { name: "Finish without service id" })),
    ).resolves.toBeUndefined();
    expect(onBlock).toHaveBeenCalledTimes(1);
  });
});
