/** Deliver a login outcome only while its screen and captured session still own it.
 *
 * `current` guards the response: a late result or error from an attempt whose
 * screen unmounted or whose session moved on is dropped. Once `success` runs it
 * is the attempt itself that changes the session (persisting the new login
 * advances the generation), so a failure inside `success` is reported whenever
 * the screen is still `mounted`; otherwise a storage or verification failure
 * would leave the user on the sign-in form with no message. */
export async function runAuthAttempt<T>(
  request: () => Promise<T>,
  current: () => boolean,
  success: (result: T) => Promise<void>,
  failure: (error: unknown) => void,
  mounted: () => boolean = current,
): Promise<void> {
  let result: T;
  try {
    result = await request();
  } catch (error) {
    if (current()) failure(error);
    return;
  }
  if (!current()) return;
  try {
    await success(result);
  } catch (error) {
    if (mounted()) failure(error);
  }
}
