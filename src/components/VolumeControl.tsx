import { useCallback, useEffect, useRef } from "react";

interface VolumeControlProps {
  volume: number;
  setVolumeValue: (value: number) => Promise<void>;
}

export function VolumeControl({ volume, setVolumeValue }: VolumeControlProps) {
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleVolumeChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const value = Number(e.target.value);

      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }

      debounceRef.current = setTimeout(() => {
        void setVolumeValue(value);
      }, 50);
    },
    [setVolumeValue],
  );

  useEffect(
    () => () => {
      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }
    },
    [],
  );

  return (
    <div className="volume-control">
      <span className="volume-icon">🔊</span>
      <input
        type="range"
        id="volume-slider"
        min="0"
        max="100"
        value={volume}
        onChange={handleVolumeChange}
      />
      <span id="volume-value">{volume}%</span>
    </div>
  );
}
