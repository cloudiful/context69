import { useEffect, useState } from "react";

/**
 * A local one-second clock while at least one visible row is unfinished, so
 * elapsed time advances without polling the API. Paused, waiting and
 * backoff rows tick too because their elapsed time is real wall time; terminal
 * rows render their finish and never need the tick.
 */
export function useElapsedNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!active) {
      return;
    }
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, [active]);

  return now;
}
