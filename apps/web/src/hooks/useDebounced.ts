import { useEffect, useState } from "react";

/**
 * `value`, once it stayed the same for `delay` milliseconds (typing pauses). Pass a primitive: a
 * new object on every render would never settle.
 */
export function useDebounced<T>(value: T, delay: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return settled;
}
