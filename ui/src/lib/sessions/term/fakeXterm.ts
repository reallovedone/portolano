// Inert stand-in for xterm.js in tests that mount the whole app: jsdom has no layout, canvas or
// `matchMedia`, which the real terminal needs. `TerminalView.test.ts` has its own scriptable fake.

export class FakeTerminal {
  cols = 80;
  options: Record<string, unknown> = {};
  rows = 24;
  open(): void {}
  loadAddon(): void {}
  onData(): { dispose(): void } {
    return { dispose: () => undefined };
  }
  onBinary(): { dispose(): void } {
    return { dispose: () => undefined };
  }
  onResize(): { dispose(): void } {
    return { dispose: () => undefined };
  }
  attachCustomKeyEventHandler(): void {}
  write(_data: unknown, done?: () => void): void {
    done?.();
  }
  hasSelection(): boolean {
    return false;
  }
  getSelection(): string {
    return "";
  }
  clearSelection(): void {}
  paste(): void {}
  focus(): void {}
  dispose(): void {}
}
