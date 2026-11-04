import { useState } from "react";
import {
  act,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createBrowserHistory, useRouter } from "@tanstack/react-router";
import { afterEach, expect, it, vi } from "vitest";
import { DialogTitle } from "./dialog";
import { Form, FormControl, FormField, useAppForm } from "./form";
import { GroupedList, GroupedInput } from "./grouped-list";
import { GroupedField, GroupedLabel, GroupedMessage } from "./grouped-fields";
import {
  GroupedFormActions,
  GroupedDialogActions,
  GroupedFormBar,
  GroupedFormScope,
} from "./grouped-form";
import {
  GroupedDialog,
  GroupedDialogContent,
  GroupedPage,
  GroupedSheet,
  GroupedSheetContent,
} from "./grouped-dialog";
import { SheetTitle } from "./sheet";

vi.mock("@tanstack/react-router", async (original) => ({
  ...(await original<typeof import("@tanstack/react-router")>()),
  useRouter: vi.fn(),
}));
afterEach(() => {
  cleanup();
  vi.mocked(useRouter).mockReset();
});

function Draft({ label = "Name", inline = false, create = false }: { readonly label?: string; readonly inline?: boolean; readonly create?: boolean }) {
  const Actions = inline ? GroupedFormActions : GroupedDialogActions;
  const form = useAppForm({ mode: "onChange", defaultValues: { name: "Original" } });
  return (
    <Form {...form}>
      <form onSubmit={form.handleSubmit(() => {})}>
        <GroupedList title="Profile">
          <FormField
            control={form.control}
            name="name"
            rules={{ required: "Enter a name" }}
            render={({ field }) => (
              <GroupedField>
                <GroupedLabel>{label}</GroupedLabel>
                <FormControl>
                  <GroupedInput {...field} />
                </FormControl>
                <GroupedMessage />
              </GroupedField>
            )}
          />
        </GroupedList>
        <Actions
          {...(!inline ? { intent: create ? "create" as const : "edit" as const } : {})}
          disabled={!form.formState.isValid}
          label={label}
          dirty={form.formState.isDirty}
          saveLabel={create ? "Create profile" : "Save changes"}
          save={form.handleSubmit(() => {})}
          discard={() => form.reset()}
        />
      </form>
    </Form>
  );
}
function Editor({ sheet = false, create = false }: { readonly sheet?: boolean; readonly create?: boolean }) {
  const [open, setOpen] = useState(true);
  return sheet ? (
    <GroupedSheet open={open} onOpenChange={setOpen}>
      <GroupedSheetContent>
        <SheetTitle>Edit profile</SheetTitle>
        <Draft inline />
      </GroupedSheetContent>
    </GroupedSheet>
  ) : (
    <GroupedDialog open={open} onOpenChange={setOpen}>
      <GroupedDialogContent>
        <DialogTitle>Edit profile</DialogTitle>
        <Draft create={create} />
      </GroupedDialogContent>
    </GroupedDialog>
  );
}

it.each(["X", "Escape", "overlay", "Cancel"])(
  "keeps a dirty dialog draft when dismissed with %s until Discard",
  async (method) => {
    const user = userEvent.setup();
    render(<Editor />);
    expect(
      screen.queryByRole("region", { name: "Unsaved changes" }),
    ).not.toBeInTheDocument();
    await user.type(screen.getByRole("textbox", { name: "Name" }), " draft");
    if (method === "X")
      await user.click(screen.getByRole("button", { name: "Close" }));
    else if (method === "Escape") await user.keyboard("{Escape}");
    else if (method === "Cancel") await user.click(screen.getByRole("button", { name: "Cancel" }));
    else
      await user.click(
        document.querySelector('[data-state="open"].fixed.inset-0')!,
      );
    const guard = await screen.findByRole("dialog", {
      name: "Discard unsaved changes?",
    });
    await user.click(
      within(guard).getByRole("button", { name: "Keep editing" }),
    );
    expect(screen.getByRole("textbox", { name: "Name" })).toHaveValue(
      "Original draft",
    );
    await user.keyboard("{Escape}");
    await user.click(
      within(
        await screen.findByRole("dialog", { name: "Discard unsaved changes?" }),
      ).getByRole("button", { name: "Discard" }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("dialog", { name: "Edit profile" }),
      ).not.toBeInTheDocument(),
    );
  },
);

it("closes a pristine dialog immediately", async () => {
  render(<Editor />);
  await userEvent.keyboard("{Escape}");
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
});

it("guards sheet Escape and restores the draft after Keep editing", async () => {
  render(<Editor sheet />);
  await userEvent.type(screen.getByRole("textbox", { name: "Name" }), " draft");
  await userEvent.keyboard("{Escape}");
  await userEvent.click(
    within(
      await screen.findByRole("dialog", { name: "Discard unsaved changes?" }),
    ).getByRole("button", { name: "Keep editing" }),
  );
  expect(screen.getByRole("textbox", { name: "Name" })).toHaveValue(
    "Original draft",
  );
});

it("guards route Back through a nested dialog and leaves its independent save bar inside the dialog", async () => {
  window.history.replaceState(null, "", "/assistant");
  const history = createBrowserHistory();
  history.push("/assistant/machines");
  history.flush();
  vi.mocked(useRouter).mockReturnValue({ history } as ReturnType<
    typeof useRouter
  >);
  render(
    <GroupedPage>
      <GroupedFormScope>
        <Draft label="Page name" inline />
        <Editor />
        <GroupedFormBar />
      </GroupedFormScope>
    </GroupedPage>,
  );
  const dialog = screen.getByRole("dialog", { name: "Edit profile" });
  await userEvent.type(
    within(dialog).getByRole("textbox", { name: "Name" }),
    " draft",
  );
  expect(
    within(dialog).getAllByRole("region", { name: "Form actions" }),
  ).toHaveLength(1);
  expect(
    screen.getAllByRole("region", { name: "Form actions" }),
  ).toHaveLength(1);
  await act(async () => history.back());
  const guard = await screen.findByRole("dialog", {
    name: "Discard unsaved changes?",
  });
  expect(history.location.pathname).toBe("/assistant/machines");
  await userEvent.click(within(guard).getByRole("button", { name: "Discard" }));
  await waitFor(() => expect(history.location.pathname).toBe("/assistant"));
  history.destroy();
});


it("shows an enabled primary create action for valid pristine prefilled defaults", async () => {
  render(<Editor create />);
  const save = screen.getByRole("button", { name: "Create profile" });
  await waitFor(() => expect(save).toBeEnabled());
  expect(save).toHaveClass("nyx-gradient-vivid");
  expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
  expect(screen.queryByText(/^Unsaved:/)).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Discard" })).not.toBeInTheDocument();
});

it("keeps edit Save visible and disabled until valid changes exist", async () => {
  render(<Editor />);
  const save = screen.getByRole("button", { name: "Save changes" });
  expect(save).toBeDisabled();
  await userEvent.type(screen.getByRole("textbox", { name: "Name" }), " updated");
  await waitFor(() => expect(save).toBeEnabled());
  await userEvent.clear(screen.getByRole("textbox", { name: "Name" }));
  await waitFor(() => expect(save).toBeDisabled());
});
