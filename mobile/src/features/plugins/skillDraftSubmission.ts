/** Validation is a separate request; losing the reviewed context must stop save. */
export async function submitSkillDraft<T extends { valid: boolean }>({
  canContinue,
  validate,
  save,
}: {
  canContinue: () => boolean;
  validate: () => Promise<T>;
  save?: () => Promise<unknown>;
}): Promise<T> {
  const assertCurrent = () => {
    if (!canContinue())
      throw new Error(
        "The draft review or app connection changed. Reload and validate again before saving.",
      );
  };
  assertCurrent();
  const result = await validate();
  assertCurrent();
  if (!result.valid) throw new Error("The draft did not pass validation.");
  if (save) await save();
  return result;
}
