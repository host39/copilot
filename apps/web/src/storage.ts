// `localStorage` access that never throws. Storage can be missing or blocked (private windows,
// site data turned off); callers then keep their values in memory for this page only.

function store(): Storage | undefined {
  try {
    return globalThis.localStorage ?? undefined;
  } catch {
    return undefined;
  }
}

export function readItem(key: string): string | null {
  try {
    return store()?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

/** Writes `value`; returns `false` when the browser refused, so the caller knows it will not persist. */
export function writeItem(key: string, value: string): boolean {
  try {
    const storage = store();
    if (!storage) return false;
    storage.setItem(key, value);
    return true;
  } catch {
    return false;
  }
}

export function removeItem(key: string): void {
  try {
    store()?.removeItem(key);
  } catch {
    // Nothing stored, nothing to remove.
  }
}
