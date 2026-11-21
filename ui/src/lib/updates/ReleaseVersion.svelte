<!-- One version of the changelog: its heading, its screenshots (if the version has any) and its
     notes. Shared by "What's new" and "Release notes". -->
<script lang="ts">
  import type { ReleaseSection } from "../ipc";
  import { versionHeading } from "./format";
  import ReleaseNotes from "./ReleaseNotes.svelte";
  import ReleaseShot from "./ReleaseShot.svelte";
  import { screenshotsFor, type ShotTheme } from "./screenshots";

  let { section, theme }: { section: ReleaseSection; theme: ShotTheme } = $props();
</script>

<section data-testid="release-section" data-version={section.version}>
  <h3>{versionHeading(section.version, section.date)}</h3>
  {#each screenshotsFor(section.version) as shot (shot.name)}
    <ReleaseShot version={section.version} {shot} {theme} />
  {/each}
  <ReleaseNotes notes={section.notes} bounded={false} />
</section>

<style>
  section + :global(section) {
    margin-top: var(--space-4);
  }

  h3 {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
  }

  /* The list already has the frame: the notes inside only add their text. */
  section :global(.notes) {
    padding: 0;
    border: 0;
    background: none;
  }
</style>
