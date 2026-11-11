/** Deliver a login outcome only while its screen and captured session still own it. */
export async function runAuthAttempt<T>(
  request: () => Promise<T>,
  current: () => boolean,
  success: (result: T) => Promise<void>,
  failure: (error: unknown) => void,
): Promise<void> {
  try {
    const result = await request();
    if (!current()) return;
    await success(result);
  } catch (error) {
    if (current()) failure(error);
  }
}
