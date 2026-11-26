// Action for a vertically scrolling box without a frame: fades the edge that has hidden content,
// so the box does not look cut off. The fade itself is the `[data-fade-*]` rule in app.css.
import { scrollEdges } from "../tabs/overflow";

export function fadeEdges(node: HTMLElement): { destroy: () => void } {
  const update = (): void => {
    const { start, end } = scrollEdges(node.scrollTop, node.clientHeight, node.scrollHeight);
    node.toggleAttribute("data-fade-top", start);
    node.toggleAttribute("data-fade-bottom", end);
  };

  node.addEventListener("scroll", update, { passive: true });
  // The box resizes with the window; its content changes without resizing it.
  const resize = new ResizeObserver(update);
  resize.observe(node);
  const mutation = new MutationObserver(update);
  mutation.observe(node, { childList: true, subtree: true, characterData: true });
  update();

  return {
    destroy() {
      node.removeEventListener("scroll", update);
      resize.disconnect();
      mutation.disconnect();
    },
  };
}
