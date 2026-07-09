import { describe, expect, it } from "vitest";
import {
  act,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
} from "@testing-library/react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { Switch } from "@/components/ui/switch";
import { Form, FormSubmitErrors, useAppForm } from "./form";

interface TestValues {
  enabled: boolean;
  name: string;
}

const defaultValues: TestValues = { enabled: false, name: "" };

describe("useAppForm", () => {
  it("marks the field dirty and touched on plain setValue", () => {
    const { result } = renderHook(() => useAppForm<TestValues>({ defaultValues }));
    // Read isDirty before the update so the formState proxy subscribes to it.
    expect(result.current.formState.isDirty).toBe(false);

    act(() => {
      result.current.setValue("enabled", true);
    });

    expect(result.current.formState.isDirty).toBe(true);
    expect(result.current.getFieldState("enabled").isDirty).toBe(true);
    expect(result.current.getFieldState("enabled").isTouched).toBe(true);
  });

  it("returns to pristine when the value matches the default again", () => {
    const { result } = renderHook(() => useAppForm<TestValues>({ defaultValues }));
    expect(result.current.formState.isDirty).toBe(false);

    act(() => {
      result.current.setValue("enabled", true);
    });
    expect(result.current.formState.isDirty).toBe(true);

    act(() => {
      result.current.setValue("enabled", false);
    });
    expect(result.current.formState.isDirty).toBe(false);
  });

  it("lets programmatic writes opt out per call", () => {
    const { result } = renderHook(() => useAppForm<TestValues>({ defaultValues }));
    expect(result.current.formState.isDirty).toBe(false);

    act(() => {
      result.current.setValue("enabled", true, {
        shouldDirty: false,
        shouldTouch: false,
      });
    });

    expect(result.current.formState.isDirty).toBe(false);
    expect(result.current.getFieldState("enabled").isDirty).toBe(false);
    expect(result.current.getFieldState("enabled").isTouched).toBe(false);
    expect(result.current.getValues("enabled")).toBe(true);
  });

  // Regression shape for the dirty-gated submit bug: a non-text control
  // (Radix Switch) wired via watch + setValue must enable a
  // disabled={!isDirty} submit button, which raw useForm never did.
  it("enables a dirty-gated submit button when a Switch is toggled", () => {
    function EditForm() {
      const form = useAppForm<TestValues>({ defaultValues });
      return (
        <form>
          <Switch
            checked={form.watch("enabled") ?? false}
            onCheckedChange={(v) => form.setValue("enabled", v)}
          />
          <button type="submit" disabled={!form.formState.isDirty}>
            Save Changes
          </button>
        </form>
      );
    }
    render(<EditForm />);

    const save = screen.getByRole("button", { name: "Save Changes" });
    expect(save).toBeDisabled();

    fireEvent.click(screen.getByRole("switch"));

    expect(save).toBeEnabled();
  });

  it("survives re-renders without double-patching setValue", () => {
    const { result, rerender } = renderHook(() =>
      useAppForm<TestValues>({ defaultValues }),
    );
    const firstSetValue = result.current.setValue;
    rerender();
    expect(result.current.setValue).toBe(firstSetValue);

    act(() => {
      result.current.setValue("name", "nyx");
    });
    expect(result.current.getFieldState("name").isDirty).toBe(true);
  });
});

describe("FormSubmitErrors", () => {
  const schema = z.object({
    name: z.string().min(1, "Name is required"),
    enabled: z.boolean(),
  });

  function TestForm() {
    const form = useAppForm<TestValues>({
      resolver: zodResolver(schema),
      defaultValues,
    });
    return (
      <Form {...form}>
        <form onSubmit={form.handleSubmit(() => undefined)}>
          <input
            aria-label="name"
            value={form.watch("name")}
            onChange={(e) => form.setValue("name", e.target.value)}
          />
          <FormSubmitErrors />
          <button type="submit">Save Changes</button>
        </form>
      </Form>
    );
  }

  it("stays hidden before any submit attempt", () => {
    render(<TestForm />);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("reports a blocked submit at the button and clears once fixed", async () => {
    render(<TestForm />);

    fireEvent.click(screen.getByRole("button", { name: "Save Changes" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Cannot save");
    expect(alert.textContent).toContain("Name is required");

    fireEvent.change(screen.getByLabelText("name"), {
      target: { value: "fixed" },
    });
    await waitFor(() => {
      expect(screen.queryByRole("alert")).toBeNull();
    });
  });
});
