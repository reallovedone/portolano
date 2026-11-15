// "Overlay open" flag: true while any menu, dialog or unlock screen is shown. Web views
// (child webviews always render above the HTML UI) and anything else that must not overlap
// the UI observe `overlay.open`.

export class OverlayStore {
  #tokens = $state<symbol[]>([]);

  get open(): boolean {
    return this.#tokens.length > 0;
  }

  /** Registers an open overlay; call the returned function to release it (idempotent). */
  acquire(): () => void {
    const token = Symbol("overlay");
    this.#tokens = [...this.#tokens, token];
    return () => {
      this.#tokens = this.#tokens.filter((t) => t !== token);
    };
  }
}
