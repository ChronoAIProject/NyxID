import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { RemoteMachineProgress } from "./remote-machine-lifecycle";
import {
  remoteMachineFixture,
  remoteMachineOperationFixture,
} from "@/test/remote-machine-fixture";

afterEach(cleanup);

it.each([
  ["create", "Setting up your machine", "Preparing workspace"],
  ["pause", "Pausing your machine", "Stopping active tasks"],
  ["resume", "Resuming your machine", "Starting runtime"],
  ["delete", "Deleting your machine", "Stopping runtime"],
] as const)(
  "shows conservative %s progress from accepted evidence",
  (action, title, middle) => {
    render(
      <RemoteMachineProgress
        machine={remoteMachineFixture}
        operation={{
          ...remoteMachineOperationFixture,
          action,
          state: "applying",
        }}
        onCheck={vi.fn()}
        onActivity={vi.fn()}
      />,
    );
    const card = within(
      screen.getByRole("region", { name: "Operation progress" }),
    );
    expect(card.getByText(title)).toBeVisible();
    expect(card.getAllByRole("listitem")).toHaveLength(3);
    expect(card.getByText(middle).closest("li")).toHaveAttribute(
      "data-state",
      "active",
    );
    expect(card.getAllByLabelText("Confirmed")).toHaveLength(1);
    expect(card.getByRole("button", { name: "View activity" })).toBeVisible();
    expect(
      card.queryByRole("button", { name: "Check status" }),
    ).not.toBeInTheDocument();
  },
);

it("does not mark request acceptance when the operation read is unavailable", () => {
  render(
    <RemoteMachineProgress
      machine={remoteMachineFixture}
      readError="Operation status unavailable"
      onCheck={vi.fn()}
    />,
  );
  expect(screen.queryByLabelText("Confirmed")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Check status" })).toBeVisible();
});

it("keeps reconciling separate from failure and never advances steps from elapsed time", () => {
  render(
    <RemoteMachineProgress
      machine={{ ...remoteMachineFixture, resource_state: "reconciling" }}
      operation={{
        ...remoteMachineOperationFixture,
        action: "resume",
        state: "reconciling",
        deadline_at: "2020-01-01T00:00:00Z",
      }}
      onCheck={vi.fn()}
    />,
  );
  expect(screen.getByText("Taking longer than usual")).toBeVisible();
  expect(screen.getAllByLabelText("Confirmed")).toHaveLength(1);
  expect(screen.getByRole("button", { name: "Check status" })).toBeVisible();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});
