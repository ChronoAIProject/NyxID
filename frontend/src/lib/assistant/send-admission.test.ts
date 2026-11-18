import { expect, it } from "vitest";
import { AccountChangedError } from "./identity";
import { isAdmittedSendError, markAdmitted } from "./send-admission";

it("never marks an account abort as an admitted send, even if previously decorated", () => {
  const abort = new AccountChangedError();
  expect(markAdmitted(abort)).toBe(abort);
  expect(abort).not.toHaveProperty("admitted");
  expect(isAdmittedSendError(Object.assign(abort, { admitted: true }))).toBe(
    false,
  );
  expect(isAdmittedSendError(markAdmitted(new Error("Lost reply")))).toBe(true);
});
