import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { MachineApplicationStatus } from "./machine-application-status";
import { machineContextReviewFixture } from "@/test/machine-access-fixture";

const pending = machineContextReviewFixture.machine_configuration!;
const applied = { ...pending, state: "applied", reason: null };
it("shows Agent authority ACK without presenting hosted pending evidence as applied", () => {
  render(
    <MachineApplicationStatus
      authorizationApplied
      environmentApplied={false}
      configuration={pending}
    />,
  );
  expect(screen.getByText("Agent authorization applied")).toBeVisible();
  expect(screen.getByText(/Hosted configuration pending/)).toBeVisible();
  expect(screen.getByText(/configuration revision 3/i)).toBeVisible();
  expect(screen.getByText(/runtime generation 4/i)).toBeVisible();
  expect(
    screen.queryByText("Environment mode applied"),
  ).not.toBeInTheDocument();
});
it.each([
  [false, false],
  [false, true],
  [true, false],
])(
  "does not invent final application from partial signals %s/%s",
  (authorizationApplied, environmentApplied) => {
    render(
      <MachineApplicationStatus
        authorizationApplied={authorizationApplied}
        environmentApplied={environmentApplied}
        configuration={applied}
      />,
    );
    expect(screen.getByText("Hosted configuration applied")).toBeVisible();
    expect(
      screen.queryByText("Environment mode applied"),
    ).not.toBeInTheDocument();
  },
);
it("shows final application only with both server-confirmed chains and the matching reviewed generation", () => {
  const { rerender } = render(
    <MachineApplicationStatus
      authorizationApplied
      environmentApplied
      configuration={applied}
      target={machineContextReviewFixture.machine_configuration_target}
    />,
  );
  expect(screen.getByText("Environment mode applied")).toBeVisible();
  rerender(
    <MachineApplicationStatus
      authorizationApplied
      environmentApplied
      configuration={{ ...applied, runtime_generation: 5 }}
      target={machineContextReviewFixture.machine_configuration_target}
    />,
  );
  expect(
    screen.queryByText("Environment mode applied"),
  ).not.toBeInTheDocument();
  expect(
    screen.getByText(
      /current hosted configuration differs from the reviewed target/i,
    ),
  ).toBeVisible();
});
it("leaves missing and unknown configuration observations unconfirmed", () => {
  const { rerender } = render(
    <MachineApplicationStatus
      authorizationApplied
      environmentApplied
      configuration={null}
    />,
  );
  expect(
    screen.queryByText("Environment mode applied"),
  ).not.toBeInTheDocument();
  rerender(
    <MachineApplicationStatus
      authorizationApplied
      environmentApplied
      configuration={{ ...applied, state: "future_state" }}
    />,
  );
  expect(
    screen.queryByText("Hosted configuration applied"),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText("Environment mode applied"),
  ).not.toBeInTheDocument();
});

it("treats ordinary Self-hosted configuration as inapplicable without inventing environment application", () => {
  const { rerender } = render(<MachineApplicationStatus authorizationApplied environmentApplied={false} configuration={null} configurationApplicability="not_applicable" />);
  expect(screen.queryByText(/Hosted configuration evidence/)).not.toBeInTheDocument();
  expect(screen.queryByText("Environment mode applied")).not.toBeInTheDocument();
  expect(screen.getByText(/Environment mode application is not confirmed/)).toBeVisible();
  rerender(<MachineApplicationStatus authorizationApplied environmentApplied configuration={null} configurationApplicability="not_applicable" />);
  expect(screen.getByText("Environment mode applied")).toBeVisible();
});
it("keeps missing Remote configuration and stale reviewed generations unconfirmed", () => {
  render(<MachineApplicationStatus authorizationApplied environmentApplied configuration={null} configurationApplicability="hosted" target={machineContextReviewFixture.machine_configuration_target} />);
  expect(screen.getByText(/not reported for this Remote machine/)).toBeVisible();
  expect(screen.getByText(/differs from the reviewed target/)).toBeVisible();
  expect(screen.queryByText("Environment mode applied")).not.toBeInTheDocument();
});
