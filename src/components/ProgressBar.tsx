import { useCallback, useMemo } from "react";

interface ProgressBarProps {
  position: number;
  duration: number;
  onSeek: (position: number) => Promise<void>;
}

export function ProgressBar({ position, duration, onSeek }: ProgressBarProps) {
  const handleProgressChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const percentage = Number(e.target.value);
      // Convert percentage to milliseconds
      const positionMs = (percentage / 100) * (duration || 1);
      void onSeek(positionMs);
    },
    [duration, onSeek]
  );

  const progressPercentage = useMemo(() => {
    if (!duration || duration === 0) return 0;
    return (position / duration) * 100;
  }, [position, duration]);

  const formatTime = (ms: number): string => {
    if (!ms || isNaN(ms)) return "0:00";
    const totalSeconds = Math.floor(ms / 1000);
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return `${minutes}:${seconds.toString().padStart(2, "0")}`;
  };

  return (
    <div className="progress-bar">
      <div className="time-current">{formatTime(position)}</div>
      <input
        type="range"
        id="progress-slider"
        min="0"
        max="100"
        value={progressPercentage}
        onChange={handleProgressChange}
      />
      <div className="time-duration">{formatTime(duration)}</div>
    </div>
  );
}
