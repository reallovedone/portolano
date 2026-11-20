// The user's appearance choices. A change applies at once (attributes of <html>, which tokens.css
// reacts to; open terminals follow the font size), is copied to localStorage for the next start's
// first paint, and is saved in `settings.json`. A failed save puts the previous look back.

import type { Appearance, Ipc } from "../ipc";
import { applyAppearance, loadStoredAppearance, normalizeAppearance, saveStoredAppearance } from "../settings/appearance";
import { t } from "../i18n/strings";
import type { NoticeStore } from "./notices.svelte";

export class AppearanceStore {
  /** What is on screen now. Starts from the localStorage copy, like the first paint did. */
  value = $state<Appearance>(loadStoredAppearance());
  readonly #ipc: Ipc;
  readonly #notices: NoticeStore;
  /** Changes are saved one after the other, so the last one chosen is the last one saved. */
  #saving: Promise<unknown> = Promise.resolve();
  /** Counts what was shown, to tell whether a newer choice replaced one that failed to save. */
  #shown = 0;

  constructor(ipc: Ipc, notices: NoticeStore) {
    this.#ipc = ipc;
    this.#notices = notices;
  }

  /** Aligns the page with `settings.json` (the truth), at start-up. Keeps the copy if it fails. */
  async load(): Promise<void> {
    const before = this.#shown;
    try {
      const stored = normalizeAppearance(await this.#ipc.appearanceGet());
      // A choice made while the file was being read is newer: keep it.
      if (this.#shown === before) this.#show(stored);
    } catch (e) {
      this.#notices.error(t("appearance.loadFailed"), e);
    }
  }

  /** Applies `change` at once and saves it. Returns false (and restores the look) if saving fails. */
  async update(change: Partial<Appearance>): Promise<boolean> {
    const previous = this.value;
    const next = normalizeAppearance({ ...previous, ...change });
    this.#show(next);
    const mine = this.#shown;
    const task = this.#saving.then(async () => {
      try {
        await this.#ipc.appearanceSet(next);
        return true;
      } catch (e) {
        // Only undo what is still on screen: a newer choice made meanwhile stays.
        if (this.#shown === mine) this.#show(previous);
        this.#notices.error(t("appearance.saveFailed"), e);
        return false;
      }
    });
    this.#saving = task;
    return task;
  }

  #show(appearance: Appearance): void {
    this.#shown += 1;
    this.value = appearance;
    applyAppearance(appearance);
    saveStoredAppearance(appearance);
  }
}
