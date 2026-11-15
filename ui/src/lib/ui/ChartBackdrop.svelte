<!-- Decorative backdrop inspired by portolan charts: sixteen points on a circle joined by
     rhumb lines, with a few well-known port numbers as bearings. Purely ornamental: it sits
     behind empty areas (welcome screen, empty states), never behind running text, ignores the
     pointer and is hidden from assistive technology. The parent must be positioned. -->
<script lang="ts">
  let {
    placement = "center",
    labels = true,
  }: {
    /** `center`: rose centred and faded at the edges. `corner`: rose rising from the
     *  bottom-right corner, leaving the rest of the area clear. */
    placement?: "center" | "corner";
    /** Show the port-number bearings. */
    labels?: boolean;
  } = $props();

  const POINTS = 16;
  const RADIUS = 420;

  const nodes = Array.from({ length: POINTS }, (_, i) => {
    const angle = (i / POINTS) * Math.PI * 2 - Math.PI / 2;
    return { x: Math.cos(angle) * RADIUS, y: Math.sin(angle) * RADIUS, angle };
  });

  const round = (value: number): number => Math.round(value * 10) / 10;

  // Every pair of points: the classic portolan network.
  const lines: string[] = [];
  for (let a = 0; a < POINTS; a += 1) {
    for (let b = a + 1; b < POINTS; b += 1) {
      lines.push(`M${round(nodes[a].x)} ${round(nodes[a].y)}L${round(nodes[b].x)} ${round(nodes[b].y)}`);
    }
  }
  const network = lines.join("");

  // The eight principal winds, from the centre, drawn slightly stronger.
  const winds = nodes
    .filter((_, i) => i % 2 === 0)
    .map((n) => `M0 0L${round(n.x)} ${round(n.y)}`)
    .join("");

  // Bearings: ports every administrator knows, placed just outside the circle.
  const BEARINGS: Array<[index: number, port: string]> = [
    [1, "22"],
    [4, "443"],
    [7, "3389"],
    [10, "5900"],
    [13, "23"],
  ];
  const bearings = BEARINGS.map(([index, port]) => {
    const n = nodes[index];
    return { port, x: round(Math.cos(n.angle) * (RADIUS + 34)), y: round(Math.sin(n.angle) * (RADIUS + 34)) };
  });
</script>

<div class="backdrop {placement}" aria-hidden="true">
  <svg viewBox="-500 -500 1000 1000" fill="none">
    <path class="net" d={network} vector-effect="non-scaling-stroke" />
    <circle class="ring" r={RADIUS} vector-effect="non-scaling-stroke" />
    <circle class="ring" r={RADIUS / 2} vector-effect="non-scaling-stroke" />
    <path class="wind" d={winds} vector-effect="non-scaling-stroke" />
    {#each nodes as n, i (i)}
      <circle class="node" cx={round(n.x)} cy={round(n.y)} r={i % 2 === 0 ? 5 : 3} />
    {/each}
    <circle class="node" r="6" />
    {#if labels}
      {#each bearings as b (b.port)}
        <text class="bearing" x={b.x} y={b.y} font-size="17" text-anchor="middle" dominant-baseline="middle">{b.port}</text>
      {/each}
    {/if}
  </svg>
</div>

<style>
  .backdrop {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
    user-select: none;
  }
  svg {
    position: absolute;
    aspect-ratio: 1;
  }
  .center svg {
    inline-size: max(100%, 100vmin);
    inset-block-start: 50%;
    inset-inline-start: 50%;
    translate: -50% -50%;
    mask-image: radial-gradient(closest-side, black 35%, transparent 100%);
  }
  .corner svg {
    inline-size: 120%;
    inset-block-end: 0;
    inset-inline-end: 0;
    translate: 46% 52%;
    mask-image: radial-gradient(closest-side, black 60%, transparent 100%);
  }
  .net {
    stroke: var(--decor-line);
    stroke-width: 1px;
  }
  .ring,
  .wind {
    stroke: var(--decor-line-strong);
    stroke-width: 1px;
  }
  .node {
    fill: var(--decor-line-strong);
  }
  .bearing {
    fill: var(--decor-text);
    font-family: var(--font-mono);
  }
</style>
