import { useCallback, useEffect, useState } from "react";

/**
 * Seconds left of a wait (a `429` answer's retry time), counting down to 0. `start(undefined)`
 * leaves it at 0.
 */
export function useCountdown(): [number, (seconds: number | undefined) => void] {
  const [until, setUntil] = useState(0);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (until <= now) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [until, now]);

  const start = useCallback((seconds: number | undefined) => {
    const current = Date.now();
    setNow(current);
    setUntil(seconds ? current + seconds * 1000 : 0);
  }, []);

  return [Math.max(0, Math.ceil((until - now) / 1000)), start];
}
