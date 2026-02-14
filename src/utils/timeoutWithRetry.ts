/**
 * Options for withTimeoutAndRetry function
 */
export interface TimeoutRetryOptions<T> {
  /** Function that returns a promise to wrap with timeout and retry logic */
  promiseFactory: () => Promise<T>;
  /** Timeout duration in milliseconds for each attempt */
  timeoutMs: number;
  /** Value to return if all retries fail */
  fallbackValue: T;
  /** Maximum number of retry attempts after the initial attempt (default: 3) */
  maxRetries?: number;
  /** Delay between retry attempts in milliseconds (default: 500) */
  retryDelayMs?: number;
  /** Callback invoked before each retry with retry number (1-indexed) */
  onRetry?: (retryNumber: number) => void;
  /** Cancellation signal to abort retries */
  signal?: AbortSignal;
}

/**
 * Wraps a promise factory with timeout and automatic retry logic. If the promise doesn't
 * resolve/reject within the specified timeout, it will automatically retry up to
 * maxRetries times after the initial attempt. Returns the fallback value if all attempts fail.
 *
 * Supports cancellation via AbortSignal - if cancelled, immediately returns fallback.
 *
 * @param options - Configuration options for timeout and retry behavior
 * @returns Promise that resolves with either the promise result or fallback value
 */
export async function withTimeoutAndRetry<T>(
  options: TimeoutRetryOptions<T>,
): Promise<T> {
  const {
    promiseFactory,
    timeoutMs,
    fallbackValue,
    maxRetries = 3,
    retryDelayMs = 500,
    onRetry,
    signal,
  } = options;

  // Check if already cancelled before starting
  if (signal?.aborted) {
    return fallbackValue;
  }

  // Total attempts = 1 initial attempt + maxRetries retry attempts
  const totalAttempts = maxRetries + 1;

  for (let attempt = 0; attempt < totalAttempts; attempt++) {
    // Check for cancellation before each attempt
    if (signal?.aborted) {
      return fallbackValue;
    }

    // Notify about retry attempt (only for retries, not the first attempt)
    // Pass retry number (1-indexed: 1st retry, 2nd retry, etc.)
    if (attempt > 0 && onRetry) {
      onRetry(attempt);
    }

    let timeoutId: number | undefined;
    let timedOut = false;

    const timeoutPromise = new Promise<T>((resolve) => {
      timeoutId = window.setTimeout(() => {
        timedOut = true;
        resolve(fallbackValue);
      }, timeoutMs);
    });

    try {
      // Create a new promise for this attempt
      const promise = promiseFactory();
      
      // Race the promise against timeout
      const result = await Promise.race([promise, timeoutPromise]);
      
      // If we got here without timing out, clear timeout and return result
      if (timeoutId !== undefined) {
        window.clearTimeout(timeoutId);
      }
      
      // If we didn't time out, return the result
      if (!timedOut) {
        return result;
      }
      
      // If we timed out and this is the last attempt or we're cancelled, return fallback
      if (attempt === totalAttempts - 1 || signal?.aborted) {
        return fallbackValue;
      }

      // Wait before next retry (unless cancelled)
      await new Promise<void>((resolve) => {
        const delayId = window.setTimeout(resolve, retryDelayMs);
        signal?.addEventListener("abort", () => {
          window.clearTimeout(delayId);
          resolve();
        }, { once: true });
      });
    } catch {
      // Clear timeout on error
      if (timeoutId !== undefined) {
        window.clearTimeout(timeoutId);
      }

      // If this is the last attempt or we're cancelled, return fallback
      if (attempt === totalAttempts - 1 || signal?.aborted) {
        return fallbackValue;
      }

      // Wait before next retry (unless cancelled)
      await new Promise<void>((resolve) => {
        const delayId = window.setTimeout(resolve, retryDelayMs);
        signal?.addEventListener("abort", () => {
          window.clearTimeout(delayId);
          resolve();
        }, { once: true });
      });
    }
  }

  return fallbackValue;
}

