import { useCallback } from "react";
import { usePlayback } from "../hooks";

interface VolumeControlProps {
  volume: number;
  setVolumeValue: (value: number) => Promise<void>;
}

export function VolumeControl({ volume, setVolumeValue }: VolumeControlProps) {
  const handleVolumeChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const value = Number(e.target.value);
      void setVolumeValue(value);
    },
    [setVolumeValue]
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
