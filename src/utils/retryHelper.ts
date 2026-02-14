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
 * @param checkFn - The async function to retry that returns true on success
 * @param initialDelayMs - Initial delay before first attempt
 * @param retryDelayMs - Delay between retry attempts
 * @param maxRetries - Maximum number of retry attempts after the initial attempt
 * @returns Promise that resolves after attempting the operation. Note: Does not
 *          indicate success/failure - checkFn may still return false after all retries.
 */
export async function retryWithDelay(
  checkFn: () => Promise<boolean>,
  initialDelayMs: number = AUTH_CHECK_INITIAL_DELAY_MS,
  retryDelayMs: number = AUTH_CHECK_RETRY_DELAY_MS,
  maxRetries: number = AUTH_CHECK_MAX_RETRIES
): Promise<void> {
  // Initial delay to allow backend to start session restoration
  await new Promise((resolve) => setTimeout(resolve, initialDelayMs));

  // Total attempts = 1 initial attempt + maxRetries retry attempts
  const totalAttempts = maxRetries + 1;
  
  for (let i = 0; i < totalAttempts; i++) {
    const success = await checkFn();
    
    if (success) {
      break; // Success, stop retrying
    }

    // Wait before next retry
    if (i < totalAttempts - 1) {
      await new Promise((resolve) => setTimeout(resolve, retryDelayMs));
    }
  }
}
