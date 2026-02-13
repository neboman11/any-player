/**
 * Wraps a promise with a timeout. If the promise doesn't resolve/reject within
 * the specified timeout, returns the fallback value instead.
 *
 * @param promise - The promise to wrap with a timeout
 * @param timeoutMs - Timeout duration in milliseconds
 * @param fallbackValue - Value to return if the promise times out
 * @returns Promise that resolves with either the promise result or fallback value
 */
export async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  fallbackValue: T,
): Promise<T> {
  let timeoutId: number | undefined;

  const timeoutPromise = new Promise<T>((resolve) => {
    timeoutId = window.setTimeout(() => resolve(fallbackValue), timeoutMs);
  });

  try {
    return await Promise.race([promise, timeoutPromise]);
  } finally {
    // Always clear the timeout after Promise.race completes to prevent memory leaks.
    // If the promise resolves first, the timeout is cleared before it fires.
    // If the timeout fires first, this cleanup is still necessary to release the timer reference.
    if (timeoutId !== undefined) {
      window.clearTimeout(timeoutId);
    }
  }
}
