import { useState } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it } from "vitest";
import { GroupedDetailContext } from "@/lib/grouped-detail";
import { SectionPages } from "./section-pages";
import { GroupedDisclosure, GroupedInput, GroupedList, GroupedRow } from "./grouped-list";
import { GroupedDraftBoundary, GroupedDraftDialog } from "./grouped-drafts";
import { GroupedFormScope, GroupedFormActions } from "./grouped-form";

const sections = ["display", "display/colors", "display/colors/text"] as const;
type Section = (typeof sections)[number];
const titles = {
  display: "Display",
  "display/colors": "Colors",
  "display/colors/text": "Text colors",
};
afterEach(cleanup);
function Editor() {
  const [draft, setDraft] = useState("");
  return (
    <GroupedList>
      <GroupedRow>
        <GroupedInput
          aria-label="Color draft"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
        />
      </GroupedRow>
      <GroupedFormActions
        label="Color draft"
        dirty={Boolean(draft)}
        save={async () => {
          setDraft("");
        }}
        discard={() => setDraft("")}
      />
    </GroupedList>
  );
}
function Tree({ deep = false }: { deep?: boolean }) {
  const [selected, setSelected] = useState<Section | undefined>(
    deep ? "display/colors/text" : undefined,
  );
  const [container, setContainer] = useState<HTMLDivElement | null>(null);
  return (
    <GroupedDraftBoundary>
      <GroupedFormScope viewport>
        <div ref={setContainer}>
          <GroupedDetailContext value={{ container, visible: true }}>
            <GroupedList>
              <GroupedDisclosure
                data-drawer-page-trigger="display"
                onClick={() => setSelected("display")}
              >
                Display
              </GroupedDisclosure>
            </GroupedList>
            <SectionPages
              sections={sections}
              titles={titles}
              selected={selected}
              rootTitle="Settings"
              navigate={setSelected}
              render={(section) =>
                section === "display/colors/text" ? (
                  <Editor />
                ) : (
                  <GroupedList>
                    <GroupedDisclosure
                      data-drawer-page-trigger={`${section}/${section === "display" ? "colors" : "text"}`}
                      onClick={() =>
                        setSelected(
                          section === "display" ? "display/colors" : "display/colors/text",
                        )
                      }
                    >
                      {section === "display" ? "Colors" : "Text colors"}
                    </GroupedDisclosure>
                  </GroupedList>
                )
              }
            />
          </GroupedDetailContext>
        </div>
        <GroupedDraftDialog />
      </GroupedFormScope>
    </GroupedDraftBoundary>
  );
}
it("mounts deep-link ancestors, walks back one level and returns focus to the originating row", async () => {
  const user = userEvent.setup();
  render(<Tree deep />);
  expect(screen.getByRole("heading", { name: "Text colors" })).toHaveFocus();
  await user.click(screen.getByRole("button", { name: "Colors" }));
  expect(screen.getByRole("button", { name: "Text colors" })).toHaveFocus();
  await user.click(screen.getByRole("button", { name: "Display" }));
  expect(screen.getByRole("button", { name: "Colors" })).toHaveFocus();
  await user.click(screen.getByRole("button", { name: "Settings" }));
  expect(screen.getByRole("button", { name: "Display" })).toHaveFocus();
});
it("retains visited drafts and applies the page guard to Back", async () => {
  const user = userEvent.setup();
  render(<Tree deep />);
  await user.type(screen.getByRole("textbox", { name: "Color draft" }), "draft");
  await user.click(screen.getByRole("button", { name: "Colors" }));
  expect(screen.getByRole("dialog", { name: "Discard unsaved changes?" })).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Keep editing" }));
  expect(screen.getByRole("textbox", { name: "Color draft" })).toHaveValue("draft");
  await user.click(screen.getByRole("button", { name: "Colors" }));
  await user.click(screen.getByRole("button", { name: "Discard" }));
  await user.click(screen.getByRole("button", { name: "Text colors" }));
  expect(screen.getByRole("textbox", { name: "Color draft" })).toHaveValue("");
});
