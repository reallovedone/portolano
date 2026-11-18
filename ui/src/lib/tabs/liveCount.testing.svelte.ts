// Test helper: records every value `liveCount()` takes inside a reactive effect, to prove the
// count is read reactively. (Runes only compile in `.svelte.ts` files.)

import { flushSync } from "svelte";

import type { TabsStore } from "./store.svelte";

export function trackLiveCount(tabs: TabsStore): { seen: number[]; stop: () => void } {
  const seen: number[] = [];
  const stop = $effect.root(() => {
    $effect(() => {
      seen.push(tabs.liveCount());
    });
  });
  flushSync();
  return { seen, stop };
}
