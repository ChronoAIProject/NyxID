import assert from "node:assert/strict";
import test from "node:test";
import { submitSkillDraft } from "./skillDraftSubmission";

test("validating alone does not save; a current validated draft saves once", async () => {
  let saves = 0;
  const validate = async () => ({ valid: true, bytes: 42 });
  const save = async () => {
    saves++;
  };
  assert.equal(
    (await submitSkillDraft({ canContinue: () => true, validate })).bytes,
    42,
  );
  assert.equal(saves, 0);
  await submitSkillDraft({ canContinue: () => true, validate, save });
  assert.equal(saves, 1);
});

test("loss of permission, account, focus or connection during validation prevents PUT", async () => {
  for (const reason of ["permission", "account", "focus", "connection"]) {
    let current = true;
    let finish!: (result: { valid: boolean }) => void;
    let saves = 0;
    const pending = submitSkillDraft({
      canContinue: () => current,
      validate: () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
      save: async () => {
        saves++;
      },
    });
    current = false;
    finish({ valid: true });
    await assert.rejects(pending, /review or app connection changed/, reason);
    assert.equal(saves, 0, reason);
  }
});

test("a background interruption stays invalid even if foreground returns before validation", async () => {
  let revision = 0;
  const started = revision;
  let finish!: (result: { valid: boolean }) => void;
  let saves = 0;
  const pending = submitSkillDraft({
    canContinue: () => revision === started,
    validate: () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
    save: async () => {
      saves++;
    },
  });
  revision++;
  finish({ valid: true });
  await assert.rejects(pending, /changed/);
  assert.equal(saves, 0);
});

test("unavailable context and failed validation never dispatch save", async () => {
  let validations = 0;
  let saves = 0;
  const validate = async () => {
    validations++;
    return { valid: false };
  };
  const save = async () => {
    saves++;
  };
  await assert.rejects(
    submitSkillDraft({ canContinue: () => false, validate, save }),
    /changed/,
  );
  assert.equal(validations, 0);
  await assert.rejects(
    submitSkillDraft({ canContinue: () => true, validate, save }),
    /did not pass/,
  );
  assert.equal(validations, 1);
  assert.equal(saves, 0);
});
