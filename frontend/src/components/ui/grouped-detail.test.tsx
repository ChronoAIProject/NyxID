import { useState } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it } from "vitest";
import { GroupedDetailContext } from "@/lib/grouped-detail";
import { GroupedDetail } from "./grouped-detail";
import {
  GroupedDisclosure,
  GroupedInput,
  GroupedList,
  GroupedRow,
} from "./grouped-list";

afterEach(cleanup);

function Pages() {
  const [container, setContainer] = useState<HTMLDivElement | null>(null);
  const [first, setFirst] = useState(false);
  const [second, setSecond] = useState(false);
  const [visible, setVisible] = useState(true);
  return (
    <>
      <button onClick={() => setVisible(!visible)}>Switch tab</button>
      <div ref={setContainer} role="dialog" aria-label="Details">
        <GroupedDetailContext.Provider value={{ container, visible }}>
          <GroupedList>
            <GroupedDisclosure onClick={() => setFirst(true)}>
              Open machine
            </GroupedDisclosure>
          </GroupedList>
          <GroupedDetail
            open={first}
            title="Machine"
            backLabel="Back to Access"
            onBack={() => setFirst(false)}
          >
            <GroupedList>
              <GroupedRow>
                <GroupedInput aria-label="Draft" />
              </GroupedRow>
              <GroupedDisclosure onClick={() => setSecond(true)}>
                Open version
              </GroupedDisclosure>
            </GroupedList>
            <GroupedDetail
              open={second}
              title="Version"
              backLabel="Back to machine"
              onBack={() => setSecond(false)}
            >
              <GroupedList>
                <GroupedRow title="Pinned" />
              </GroupedList>
            </GroupedDetail>
          </GroupedDetail>
        </GroupedDetailContext.Provider>
      </div>
    </>
  );
}

it("retains drafts and restores focus through nested drill-ins", async () => {
  const user = userEvent.setup();
  render(<Pages />);
  const machine = screen.getByRole("button", { name: "Open machine" });
  await user.click(machine);
  expect(screen.getByRole("heading", { name: "Machine" })).toHaveFocus();
  expect(machine.closest("section")).toHaveAttribute("aria-hidden", "true");
  await user.type(
    screen.getByRole("textbox", { name: "Draft" }),
    "Keep this draft",
  );
  const version = screen.getByRole("button", { name: "Open version" });
  await user.click(version);
  expect(screen.getByRole("heading", { name: "Version" })).toHaveFocus();
  await user.click(screen.getByRole("button", { name: "Back to machine" }));
  expect(version).toHaveFocus();
  expect(screen.getByRole("textbox", { name: "Draft" })).toHaveValue(
    "Keep this draft",
  );
  await user.click(screen.getByRole("button", { name: "Back to Access" }));
  expect(machine).toHaveFocus();
  expect(machine.closest("section")).not.toHaveAttribute("aria-hidden");
  await user.click(machine);
  expect(screen.getByRole("textbox", { name: "Draft" })).toHaveValue(
    "Keep this draft",
  );
});

it("releases shared covers when both nested pages leave the active tab", async () => {
  const user = userEvent.setup();
  render(<Pages />);
  const machine = screen.getByRole("button", { name: "Open machine" });
  await user.click(machine);
  await user.click(screen.getByRole("button", { name: "Open version" }));
  await user.click(screen.getByRole("button", { name: "Switch tab" }));
  expect(machine.closest("section")).not.toHaveAttribute("aria-hidden");
  expect(machine.closest("section")!.inert).toBeFalsy();
  await user.click(screen.getByRole("button", { name: "Switch tab" }));
  expect(screen.getByRole("heading", { name: "Version" })).toHaveFocus();
});

function ReusedPicker() {
  const [container, setContainer] = useState<HTMLDivElement | null>(null);
  const [skill, setSkill] = useState<string>();
  const [version, setVersion] = useState(false);
  return (
    <div ref={setContainer} role="dialog" aria-label="Skills">
      <GroupedDetailContext.Provider value={{ container, visible: true }}>
        <GroupedList>
          {["First", "Second"].map((name) => (
            <GroupedDisclosure key={name} onClick={() => setSkill(name)}>
              {name} skill
            </GroupedDisclosure>
          ))}
        </GroupedList>
        {["First", "Second"].map((name) => (
          <GroupedDetail
            key={name}
            open={skill === name}
            title={`${name} detail`}
            backLabel="Back to Skills"
            onBack={() => setSkill(undefined)}
          >
            <GroupedList>
              <GroupedDisclosure onClick={() => setVersion(true)}>
                Choose version
              </GroupedDisclosure>
            </GroupedList>
          </GroupedDetail>
        ))}
        <GroupedDetail
          open={version}
          level={2}
          title="Skill version"
          backLabel="Back to skill"
          onBack={() => setVersion(false)}
        >
          <GroupedList>
            <GroupedRow title="Version 2" />
          </GroupedList>
        </GroupedDetail>
      </GroupedDetailContext.Provider>
    </div>
  );
}

it("reopens a retained shared picker above a detail page mounted later", async () => {
  const user = userEvent.setup();
  render(<ReusedPicker />);
  await user.click(screen.getByRole("button", { name: "First skill" }));
  await user.click(screen.getByRole("button", { name: "Choose version" }));
  await user.click(screen.getByRole("button", { name: "Back to skill" }));
  await user.click(screen.getByRole("button", { name: "Back to Skills" }));
  await user.click(screen.getByRole("button", { name: "Second skill" }));
  const disclosure = screen.getByRole("button", { name: "Choose version" });
  await user.click(disclosure);
  const picker = screen.getByRole("heading", { name: "Skill version" });
  expect(picker).toHaveFocus();
  expect(picker.closest<HTMLElement>("[data-grouped-detail]")!.inert).toBeFalsy();
  expect(picker.closest("[data-grouped-detail]")).toHaveStyle({ zIndex: 12 });
  expect(screen.queryByRole("heading", { name: "Second detail" })).toBeNull();
  await user.click(screen.getByRole("button", { name: "Back to skill" }));
  expect(disclosure).toHaveFocus();
  await user.click(screen.getByRole("button", { name: "Back to Skills" }));
  expect(screen.getByRole("button", { name: "Second skill" })).toHaveFocus();
});
