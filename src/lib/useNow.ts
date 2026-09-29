import { useEffect, useState } from "react";

/**
 * Current unix seconds, ticking so live task durations update. Kept in one
 * place so many components can share a single interval.
 */
export function useNow(intervalMs = 1000): number {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    const t = window.setInterval(
      () => setNow(Math.floor(Date.now() / 1000)),
      intervalMs,
    );
    return () => window.clearInterval(t);
  }, [intervalMs]);
  return now;
}
