/**
 * Result type for withTimeout that explicitly indicates whether a timeout occurred
 */
export type TimeoutResult<T> = {
  /** The value returned - either from the promise or the fallback */
  value: T;
  /** True if the operation timed out, false if it completed successfully */
  timedOut: boolean;
};

/**
 * Wraps a promise with a timeout. If the promise doesn't resolve/reject within
 * the specified timeout, returns the fallback value with a timeout indicator.
 *
 * @param promise - The promise to wrap with a timeout
 * @param timeoutMs - Timeout duration in milliseconds
 * @param fallbackValue - Value to return if the promise times out
 * @returns Promise that resolves with a TimeoutResult containing the value and timeout status
 */
export async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  fallbackValue: T,
): Promise<TimeoutResult<T>> {
  let timeoutId: number | undefined;

  // Use a unique symbol to detect which promise won the race
  const TIMEOUT_SYMBOL = Symbol("timeout");

  const timeoutPromise = new Promise<T | symbol>((resolve) => {
    timeoutId = window.setTimeout(() => {
      console.warn(
        `Operation timed out after ${timeoutMs}ms, using fallback value`,
      );
      resolve(TIMEOUT_SYMBOL);
    }, timeoutMs);
  });

  try {
    const result = await Promise.race([promise, timeoutPromise]);
    const timedOut = result === TIMEOUT_SYMBOL;
    const value = timedOut ? fallbackValue : (result as T);
    return { value, timedOut };
  } finally {
    // Always clear the timeout after Promise.race completes to prevent memory leaks.
    // If the promise resolves first, the timeout is cleared before it fires.
    // If the timeout fires first, this cleanup is still necessary to release the timer reference.
    if (timeoutId !== undefined) {
      window.clearTimeout(timeoutId);
    }
  }
}
