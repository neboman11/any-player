/**
 * Retry configuration for authentication checks.
 * These values are tuned for backend session restoration timing.
 */
export const AUTH_CHECK_INITIAL_DELAY_MS = 500;
export const AUTH_CHECK_RETRY_DELAY_MS = 300;
export const AUTH_CHECK_MAX_RETRIES = 3;

/**
 * Retries an async function with configurable delays and attempts.
 * 
 * @param checkFn - The async function to retry
 * @param initialDelayMs - Initial delay before first attempt
 * @param retryDelayMs - Delay between retry attempts
 * @param maxRetries - Maximum number of retry attempts
 * @returns Promise that resolves when checkFn succeeds or max retries reached
 */
export async function retryWithDelay(
  checkFn: () => Promise<boolean>,
  initialDelayMs: number = AUTH_CHECK_INITIAL_DELAY_MS,
  retryDelayMs: number = AUTH_CHECK_RETRY_DELAY_MS,
  maxRetries: number = AUTH_CHECK_MAX_RETRIES
): Promise<void> {
  // Initial delay to allow backend to start session restoration
  await new Promise((resolve) => setTimeout(resolve, initialDelayMs));

  // Try up to maxRetries times
  for (let i = 0; i < maxRetries; i++) {
    const success = await checkFn();
    
    if (success) {
      break; // Success, stop retrying
    }

    // Wait before next retry
    if (i < maxRetries - 1) {
      await new Promise((resolve) => setTimeout(resolve, retryDelayMs));
    }
  }
}
