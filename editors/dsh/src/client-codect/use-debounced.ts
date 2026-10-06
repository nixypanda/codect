/**
 * use-debounced.ts — Return a value only after it has stayed unchanged for a
 * short delay. Used to keep free-text revision/path inputs from spawning a CLI
 * process on every keystroke.
 */

import { useEffect, useState } from "react";

/** Trailing debounce. The first render returns `value` immediately. */
export function useDebounced<T>(value: T, delayMs = 250): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(id);
  }, [value, delayMs]);
  return debounced;
}
