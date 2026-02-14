interface LoadingSpinnerProps {
  size?: "small" | "medium" | "large";
}

export function LoadingSpinner({ size = "medium" }: LoadingSpinnerProps) {
  return (
    <span
      className={`loading-spinner loading-spinner-${size}`}
      aria-hidden="true"
    />
  );
}
