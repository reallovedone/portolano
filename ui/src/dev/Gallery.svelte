<!-- Design gallery: every token and component in every state, for visual review.
     Open with `?gallery` on the dev server (`pnpm dev:mock`) or a mock build. Forced classes
     (forceHover / forceFocus) show states that normally need the pointer or the keyboard. -->
<script lang="ts">
  import { onMount } from "svelte";

  import {
    ANSI_TOKENS,
    darkTokens,
    lightTokens,
    parseColor,
    ratio,
    type TokenMap,
  } from "../lib/design/contrast";
  import { ICON_NAMES } from "../lib/design/icons/registry";
  import { initPlatform } from "../lib/design/platform";
  import tokensCss from "../lib/design/tokens.css?raw";
  import { createMockIpc } from "../lib/ipc";
  import { createApp, provideApp } from "../lib/state/app";
  import type { NodeKind } from "../lib/tree/model";
  import Badge from "../lib/ui/Badge.svelte";
  import Banner from "../lib/ui/Banner.svelte";
  import Button from "../lib/ui/Button.svelte";
  import ChartBackdrop from "../lib/ui/ChartBackdrop.svelte";
  import Checkbox from "../lib/ui/Checkbox.svelte";
  import ConfirmDialog from "../lib/ui/ConfirmDialog.svelte";
  import ContextMenu from "../lib/ui/ContextMenu.svelte";
  import Dialog from "../lib/ui/Dialog.svelte";
  import EmptyState from "../lib/ui/EmptyState.svelte";
  import FormRow from "../lib/ui/FormRow.svelte";
  import FormSection from "../lib/ui/FormSection.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import IconButton from "../lib/ui/IconButton.svelte";
  import Kbd from "../lib/ui/Kbd.svelte";
  import KindIcon from "../lib/ui/KindIcon.svelte";
  import KindTile from "../lib/ui/KindTile.svelte";
  import ListRow from "../lib/ui/ListRow.svelte";
  import Logo from "../lib/ui/Logo.svelte";
  import type { MenuItem } from "../lib/ui/menu";
  import PromptDialog from "../lib/ui/PromptDialog.svelte";
  import Radio from "../lib/ui/Radio.svelte";
  import RadioGroup from "../lib/ui/RadioGroup.svelte";
  import SegmentedControl from "../lib/ui/SegmentedControl.svelte";
  import Select from "../lib/ui/Select.svelte";
  import Separator from "../lib/ui/Separator.svelte";
  import Spinner from "../lib/ui/Spinner.svelte";
  import Switch from "../lib/ui/Switch.svelte";
  import TextArea from "../lib/ui/TextArea.svelte";
  import TextField from "../lib/ui/TextField.svelte";
  import Toasts from "../lib/ui/Toasts.svelte";
  import Toolbar from "../lib/ui/Toolbar.svelte";
  import Tooltip from "../lib/ui/Tooltip.svelte";
  import TitlebarDragRegion from "../lib/ui/TitlebarDragRegion.svelte";
  import ImportDialog from "../lib/transfer/ImportDialog.svelte";
  import Section from "./Section.svelte";

  const app = createApp(createMockIpc());
  provideApp(app);

  // ---- Chrome: theme, platform and window-activity switches ----------------------------
  let theme = $state("system");
  let platform = $state("windows");
  let inactive = $state(false);

  $effect(() => {
    const root = document.documentElement;
    if (theme === "system") delete root.dataset["theme"];
    else root.dataset["theme"] = theme;
  });
  $effect(() => {
    document.documentElement.dataset["platform"] = platform;
  });
  $effect(() => {
    document.documentElement.dataset["windowActive"] = inactive ? "false" : "true";
  });
  onMount(() => {
    const dispose = initPlatform(document.documentElement, "windows");
    return () => {
      dispose();
      delete document.documentElement.dataset["theme"];
    };
  });

  // ---- Token tables ---------------------------------------------------------------------
  const light = lightTokens(tokensCss);
  const dark = darkTokens(tokensCss);

  const COLOR_GROUPS: { title: string; prefix: RegExp }[] = [
    { title: "Surfaces", prefix: /^--surface-/ },
    { title: "Text", prefix: /^--text-(?!selection)/ },
    { title: "Borders", prefix: /^--border-/ },
    { title: "Accent and focus", prefix: /^--(accent|focus-ring$|text-selection)/ },
    { title: "Selection", prefix: /^--selection-/ },
    { title: "Danger", prefix: /^--danger-/ },
    { title: "Warning", prefix: /^--warning-/ },
    { title: "Success", prefix: /^--success-/ },
    { title: "Info", prefix: /^--info-/ },
    { title: "Protocols", prefix: /^--protocol-/ },
    { title: "Scrim", prefix: /^--scrim$/ },
    { title: "Terminal pane and letterbox", prefix: /^--(terminal-bg|letterbox-bg)$/ },
  ];

  // The ANSI palette is drawn on the terminal background of each theme (read from the token maps,
  // so both themes show at once), with the measured contrast ratio of every colour.
  const ANSI_PANELS: { title: string; tokens: TokenMap }[] = [
    { title: "Light", tokens: light },
    { title: "Dark", tokens: dark },
  ];

  function ansiRatio(tokens: TokenMap, name: string): string {
    const fg = parseColor(tokens[name] ?? "");
    const bg = parseColor(tokens["--terminal-bg"] ?? "");
    return fg && bg ? ratio(fg, bg).toFixed(2) : "?";
  }

  function colorNames(prefix: RegExp): string[] {
    return Object.keys({ ...light, ...dark }).filter(
      (name) => prefix.test(name) && !/-width$|-offset$/.test(name),
    );
  }

  const SPACES = ["1", "2", "3", "4", "5", "6", "8", "10", "12"];
  const SIZES = [
    ["xs", "11 — captions, badges"],
    ["sm", "12 — secondary text, hints"],
    ["md", "13 — body (base)"],
    ["lg", "15 — dialog titles, empty states"],
    ["xl", "20 — large titles"],
  ] as const;
  const RADIO_OPTIONS = [
    { value: "inherit", label: "Inherit from parent folder", hint: "Use the credential of the enclosing folder." },
    { value: "none", label: "None", hint: "Connect without a stored credential." },
    { value: "id", label: "Specific credential", hint: "Pick one from the vault." },
  ];
  const KINDS: NodeKind[] = ["folder", "localShell", "ssh", "rdp", "vnc", "web"];

  // ---- Component state --------------------------------------------------------------------
  let text = $state("prod-db-01");
  let password = $state("correct horse");
  let area = $state("Line one\nLine two");
  let selectValue = $state("ssh");
  let checkA = $state(true);
  let checkB = $state(false);
  let seg = $state("list");
  let segIcons = $state("a");
  let switchOn = $state(true);
  let switchOff = $state(false);
  let rowSelected = $state("prod");
  let radioMode = $state("inherit");
  let radioSingle = $state("a");
  let monoHost = $state("build-01.corp.example:3389");
  let monoKey = $state("-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA…");
  let searchText = $state("prod");
  let toolbarNote = $state("");

  const SELECT_OPTIONS = [
    { value: "ssh", label: "SSH" },
    { value: "rdp", label: "Remote Desktop (RDP)" },
    { value: "vnc", label: "VNC" },
    { value: "web", label: "Web", disabled: true },
  ];

  // ---- Overlays ---------------------------------------------------------------------------
  let dialog = $state<"none" | "sample" | "confirm" | "danger" | "prompt" | "large" | "hero" | "import">("none");
  let menu = $state<{ x: number; y: number } | null>(null);

  const MENU_ITEMS: MenuItem[] = [
    { label: "Connect", action: () => (toolbarNote = "Connect"), shortcut: "⏎", icon: "plug" },
    { label: "Edit…", action: () => (toolbarNote = "Edit"), shortcut: "⌘E", icon: "edit" },
    { label: "Duplicate", action: () => (toolbarNote = "Duplicate"), icon: "copy" },
    { label: "Move to…", action: () => (toolbarNote = "Move"), disabled: true, separatorBefore: true },
    { label: "Delete", action: () => (toolbarNote = "Delete"), danger: true, separatorBefore: true, icon: "trash" },
  ];

  function openMenu(event: MouseEvent): void {
    event.preventDefault();
    menu = { x: event.clientX, y: event.clientY };
  }

  const NAV = [
    ["colors", "Colours"],
    ["type", "Type"],
    ["layout", "Spacing and shape"],
    ["icons", "Icons"],
    ["buttons", "Buttons"],
    ["fields", "Fields"],
    ["controls", "Controls"],
    ["menus", "Menus and tooltips"],
    ["lists", "Lists"],
    ["feedback", "Feedback"],
    ["forms", "Forms"],
    ["dialogs", "Dialogs and toasts"],
    ["chrome", "Window chrome"],
    ["sample", "Sample window"],
  ];
</script>

<div class="page">
  <header class="bar">
    <div class="brand"><Logo size={20} /> <strong>Portolano design gallery</strong></div>
    <nav aria-label="Sections">
      {#each NAV as [id, label] (id)}<a href="#{id}">{label}</a>{/each}
    </nav>
    <div class="switches">
      <SegmentedControl
        label="Theme"
        size="compact"
        bind:value={theme}
        options={[
          { value: "system", label: "System" },
          { value: "light", label: "Light" },
          { value: "dark", label: "Dark" },
        ]}
      />
      <SegmentedControl
        label="Platform"
        size="compact"
        bind:value={platform}
        options={[
          { value: "windows", label: "Windows" },
          { value: "macos", label: "macOS" },
        ]}
      />
      <Checkbox label="Inactive window" bind:checked={inactive} />
    </div>
  </header>

  <main class="content">
    <!-- ======================== Colours ======================== -->
    <Section
      id="colors"
      title="Colours"
      note="Swatches follow the active theme. Names are the semantic tokens; screens must use only these."
    >
      {#each COLOR_GROUPS as group (group.title)}
        <h3 class="group">{group.title}</h3>
        <div class="swatches">
          {#each colorNames(group.prefix) as name (name)}
            <div class="swatch">
              <span class="chip" style:background="var({name})"></span>
              <code>{name}</code>
            </div>
          {/each}
        </div>
      {/each}
      <h3 class="group">Letterbox (--letterbox-bg: around a remote desktop, dark in both themes)</h3>
      <div class="letterbox">
        <div class="letterbox-desktop">remote desktop</div>
      </div>
      <h3 class="group">ANSI palette on the terminal background (ratio vs --terminal-bg, minimum 4.5)</h3>
      <div class="ansi-panels">
        {#each ANSI_PANELS as panel (panel.title)}
          <div class="ansi-panel" style:background={panel.tokens["--terminal-bg"]}>
            <p class="ansi-title" style:color={panel.tokens["--text-primary"]}>{panel.title} terminal</p>
            {#each ANSI_TOKENS as name (name)}
              <p class="ansi-row" style:color={panel.tokens[name]}>
                <span class="ansi-sample">██ $ ssh prod</span>
                <code>{name.slice(2)}</code>
                <code>{panel.tokens[name]}</code>
                <code>{ansiRatio(panel.tokens, name)}:1</code>
              </p>
            {/each}
          </div>
        {/each}
      </div>
      <h3 class="group">Shadows</h3>
      <div class="shadows">
        <div class="shadow-box" style:box-shadow="var(--shadow-control)">control</div>
        <div class="shadow-box" style:box-shadow="var(--shadow-popover)">popover</div>
        <div class="shadow-box" style:box-shadow="var(--shadow-dialog)">dialog</div>
      </div>
    </Section>

    <!-- ======================== Type ======================== -->
    <Section id="type" title="Type" note="System font stack. Scale 11 / 12 / 13 (base) / 15 / 20; weights 400 / 500 / 600.">
      {#each SIZES as [key, desc] (key)}
        <p class="type-row" style:font-size="var(--font-size-{key})">
          The quick brown fox connects to a remote host — <span class="muted">{desc}</span>
        </p>
      {/each}
      <p class="type-row">
        <span style:font-weight="var(--font-weight-regular)">Regular 400</span> ·
        <span style:font-weight="var(--font-weight-medium)">Medium 500</span> ·
        <span style:font-weight="var(--font-weight-semibold)">Semibold 600</span>
      </p>
      <p class="type-row"><code>mono: ssh admin@10.0.0.12 -p 2222</code></p>
      <p class="type-row mono-lines">
        <code>terminal line height (--terminal-line-height, 1.2):<br />$ ls -la /var/log<br />$ tail -f syslog</code>
      </p>
      <p class="type-row">
        <span class="muted">Text colours:</span>
        <span style:color="var(--text-primary)">primary</span> ·
        <span style:color="var(--text-secondary)">secondary</span> ·
        <span style:color="var(--text-tertiary)">tertiary</span> ·
        <span style:color="var(--text-disabled)">disabled</span> ·
        <a href="#type">link</a>
      </p>
    </Section>

    <!-- ======================== Layout ======================== -->
    <Section id="layout" title="Spacing and shape" note="4 px grid, radii 6 (controls) and 10 (panels), control heights 24 / 28 / 32.">
      <div class="spaces">
        {#each SPACES as n (n)}
          <div class="space">
            <span class="bar-sample" style:width="var(--space-{n})"></span>
            <code>--space-{n}</code>
          </div>
        {/each}
      </div>
      <div class="shapes">
        <div class="shape" style:border-radius="var(--radius-xs)">xs</div>
        <div class="shape" style:border-radius="var(--radius-control)">control</div>
        <div class="shape" style:border-radius="var(--radius-panel)">panel</div>
        <div class="shape" style:border-radius="var(--radius-full)">full</div>
      </div>
      <div class="row-wrap">
        <div class="height" style:height="var(--control-h-compact)">compact 24</div>
        <div class="height" style:height="var(--control-h-regular)">regular 28</div>
        <div class="height" style:height="var(--control-h-large)">large 32</div>
      </div>
    </Section>

    <!-- ======================== Icons ======================== -->
    <Section id="icons" title="Icons" note="Lucide (ISC) for the interface; kind icons are drawn in the same single-stroke style as the logo.">
      <h3 class="group">Connection kinds (sizes 14 / 16 / 20)</h3>
      <div class="row-wrap">
        {#each KINDS as kind (kind)}
          <div class="icon-cell">
            <div class="row-wrap tight">
              <KindIcon {kind} size={14} /><KindIcon {kind} size={16} /><KindIcon {kind} size={20} />
              {#if kind === "folder"}<KindIcon {kind} open size={16} />{/if}
            </div>
            <code>{kind}</code>
          </div>
        {/each}
      </div>
      <h3 class="group">Logo</h3>
      <div class="row-wrap">
        <Logo size={16} /><Logo size={24} /><Logo size={48} /><Logo size={96} />
      </div>
      <h3 class="group">Interface icons</h3>
      <div class="icon-grid">
        {#each ICON_NAMES as name (name)}
          <div class="icon-cell"><Icon {name} size={20} /><code>{name}</code></div>
        {/each}
      </div>
    </Section>

    <!-- ======================== Buttons ======================== -->
    <Section id="buttons" title="Buttons" note="Rows: variants. Columns: default, hover (forced), focus (forced), disabled, loading.">
      {#each ["primary", "secondary", "plain", "destructive"] as const as variant (variant)}
        <div class="row-wrap">
          <span class="row-label">{variant}</span>
          <Button {variant}>Default</Button>
          <Button {variant} forceHover>Hover</Button>
          <Button {variant} forceFocus>Focus</Button>
          <Button {variant} disabled>Disabled</Button>
          <Button {variant} loading>Loading</Button>
          <Button {variant}>
            {#snippet icon()}<Icon name="plug" />{/snippet}
            With icon
          </Button>
        </div>
      {/each}
      <h3 class="group">Sizes</h3>
      <div class="row-wrap">
        <Button size="compact" variant="primary">Compact 24</Button>
        <Button size="regular" variant="primary">Regular 28</Button>
        <Button size="large" variant="primary">Large 32</Button>
        <Button size="compact">Compact</Button>
        <Button size="large">Large</Button>
      </div>
      <h3 class="group">Icon buttons (hover a button for its tooltip)</h3>
      <div class="row-wrap">
        <IconButton icon="plus" label="New connection" />
        <IconButton icon="plus" label="Forced hover" forceHover />
        <IconButton icon="plus" label="Forced focus" forceFocus />
        <IconButton icon="plus" label="Disabled" disabled />
        <IconButton icon="panel-left" label="Toggle sidebar" pressed={true} />
        <IconButton icon="settings" label="Settings" variant="filled" />
        <IconButton icon="more" label="Compact" size="compact" />
        <IconButton icon="search" label="Large" size="large" />
      </div>
    </Section>

    <!-- ======================== Fields ======================== -->
    <Section id="fields" title="Fields" note="TextField, TextArea, Select. Focus uses the accent ring; errors use the danger colour and are announced.">
      <div class="grid2">
        <TextField label="Name" bind:value={text} hint="Shown in the connection tree." />
        <TextField label="Focused (forced)" value="Focus ring" forceFocus />
        <TextField label="Host" value="" placeholder="host or IP address" error="Host is required." />
        <TextField label="Disabled" value="read only" disabled />
        <TextField label="URL" value="example.com/admin" prefix="https://" />
        <TextField label="Port" value="22" suffix="TCP" />
        <TextField label="Password" type="password" bind:value={password} hint="Use the eye to reveal." />
        <TextField label="Search" type="search" placeholder="Filter" size="compact">
          {#snippet prefix()}<Icon name="search" size={14} />{/snippet}
        </TextField>
        <TextField label="Large" value="32 px high" size="large" />
        <TextField label="Tag" value="" optionalLabel="optional" placeholder="e.g. production" />
        <TextField label="Host (monospace)" bind:value={monoHost} monospace hint="monospace prop: host names, ports, paths." />
        <TextField label="Search (native cancel hidden)" type="search" bind:value={searchText} placeholder="Filter" />
      </div>
      <div class="grid2">
        <TextArea label="Notes" bind:value={area} hint="Multi-line text." />
        <TextArea label="Notes (error)" value="" error="Too short." />
        <TextArea label="Private key (monospace)" bind:value={monoKey} monospace rows={3} spellcheck="false" />
        <Select label="Protocol" bind:value={selectValue} options={SELECT_OPTIONS} hint="Native pop-up button." />
        <Select label="Protocol (error)" value="ssh" options={SELECT_OPTIONS} error="Choose a supported protocol." />
        <Select label="Disabled" value="ssh" options={SELECT_OPTIONS} disabled />
        <Select label="Focused (forced)" value="rdp" options={SELECT_OPTIONS} forceFocus />
        <Select label="Compact" value="vnc" options={SELECT_OPTIONS} size="compact" />
      </div>
    </Section>

    <!-- ======================== Controls ======================== -->
    <Section id="controls" title="Controls" note="Checkbox, Switch, SegmentedControl.">
      <div class="grid2">
        <div class="stack">
          <Checkbox label="Checked" bind:checked={checkA} />
          <Checkbox label="Unchecked" bind:checked={checkB} />
          <Checkbox label="Indeterminate" indeterminate />
          <Checkbox label="Focused (forced)" checked forceFocus />
          <Checkbox label="Disabled" disabled />
          <Checkbox label="Disabled checked" checked disabled />
          <Checkbox label="With hint" hint="Secondary description under the label." />
        </div>
        <div class="stack">
          <Switch label="Lock vault on inactivity" bind:checked={switchOn} />
          <Switch label="Off" bind:checked={switchOff} hint="Hint text under the label." />
          <Switch label="Focused (forced)" checked forceFocus />
          <Switch label="Disabled off" disabled />
          <Switch label="Disabled on" checked disabled />
        </div>
      </div>
      <h3 class="group">Radio and RadioGroup</h3>
      <div class="grid2">
        <RadioGroup label="Credential mode" options={RADIO_OPTIONS} bind:value={radioMode} idPrefix="gal-radio" />
        <div class="stack">
          <Radio name="gal-solo" value="a" label="Selected" bind:group={radioSingle} />
          <Radio name="gal-solo" value="b" label="Unselected" bind:group={radioSingle} />
          <Radio name="gal-solo" value="c" label="Focused (forced)" forceFocus bind:group={radioSingle} />
          <Radio name="gal-off" value="d" label="Disabled" disabled group="x" />
          <Radio name="gal-off2" value="e" label="Disabled selected" disabled group="e" />
          <RadioGroup label="Disabled group" options={RADIO_OPTIONS} value="none" disabled />
        </div>
      </div>
      <h3 class="group">Segmented control</h3>
      <div class="row-wrap">
        <SegmentedControl
          label="View"
          bind:value={seg}
          options={[
            { value: "list", label: "List" },
            { value: "grid", label: "Grid" },
            { value: "off", label: "Disabled", disabled: true },
          ]}
        />
        <SegmentedControl
          label="Mode"
          size="compact"
          bind:value={segIcons}
          iconOnly
          options={[
            { value: "a", label: "Terminal", icon: "terminal" },
            { value: "b", label: "Desktop", icon: "monitor" },
            { value: "c", label: "Web", icon: "globe" },
          ]}
        />
        <SegmentedControl label="Disabled group" disabled value="x" options={[{ value: "x", label: "One" }, { value: "y", label: "Two" }]} />
      </div>
    </Section>

    <!-- ======================== Menus and tooltips ======================== -->
    <Section id="menus" title="Menus and tooltips" note="Right-click the panel or press the button to open the context menu. Arrow keys, Home/End, Enter, Escape work.">
      <div class="row-wrap">
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="menu-target" oncontextmenu={openMenu}>Right-click here</div>
        <Button onclick={(e) => { const r = (e.currentTarget as HTMLElement).getBoundingClientRect(); menu = { x: r.left, y: r.bottom + 4 }; }}>Open menu</Button>
        {#if toolbarNote}<Badge variant="info">Last action: {toolbarNote}</Badge>{/if}
      </div>
      <div class="row-wrap">
        <Tooltip text="A tooltip that is always shown" open placement="bottom">
          <Button>Forced open</Button>
        </Tooltip>
        <Tooltip text="Appears on hover and on keyboard focus">
          <Button>Hover me</Button>
        </Tooltip>
        <span class="muted">Shortcut hints:</span> <Kbd>⌘N</Kbd> <Kbd>⌘⇧L</Kbd> <Kbd>Ctrl</Kbd> <Kbd>Esc</Kbd>
      </div>
    </Section>

    <!-- ======================== Lists ======================== -->
    <Section id="lists" title="Lists" note="ListRow is the building block for tree rows and lists: hover, selected, keyboard focus, leading icon, trailing accessory.">
      <div class="list-demo">
        <div class="list" role="listbox" aria-label="Sample list">
          {#each [["prod", "ssh", "prod-db-01", "admin@10.0.0.12"], ["win", "rdp", "Build server", "Administrator"], ["vnc", "vnc", "Lab console", "192.168.1.40"], ["web", "web", "Router admin", "https://192.168.1.1"]] as [id, kind, name, detail] (id)}
            <ListRow
              role="option"
              tabindex={0}
              aria-selected={rowSelected === id}
              selected={rowSelected === id}
              onclick={() => (rowSelected = id as string)}
              onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") rowSelected = id as string; }}
            >
              {#snippet leading()}<KindIcon kind={kind as NodeKind} />{/snippet}
              {name}
              {#snippet secondary()}{detail}{/snippet}
              {#snippet trailing()}{#if id === "prod"}<Badge variant="success">Connected</Badge>{/if}{/snippet}
            </ListRow>
          {/each}
        </div>
        <div class="list">
          <ListRow>Default{#snippet leading()}<Icon name="folder" />{/snippet}</ListRow>
          <ListRow forceHover>Hover (forced){#snippet leading()}<Icon name="folder" />{/snippet}</ListRow>
          <ListRow selected>Selected{#snippet leading()}<Icon name="folder" />{/snippet}</ListRow>
          <ListRow forceFocus>Keyboard focus (forced){#snippet leading()}<Icon name="folder" />{/snippet}</ListRow>
          <ListRow disabled>Disabled{#snippet leading()}<Icon name="folder" />{/snippet}</ListRow>
          <ListRow dropTarget>Drop target (dropTarget){#snippet leading()}<Icon name="folder" />{/snippet}</ListRow>
          <ListRow dragging>Being dragged (dragging){#snippet leading()}<Icon name="folder" />{/snippet}</ListRow>
          <ListRow density="comfortable">Comfortable density (tree){#snippet leading()}<KindIcon kind="ssh" size={20} />{/snippet}{#snippet secondary()}admin@10.0.0.12{/snippet}</ListRow>
          <ListRow depth={2} density="compact">Compact, depth 2{#snippet leading()}<Icon name="chevron-right" size={14} />{/snippet}{#snippet trailing()}<Kbd>⌘1</Kbd>{/snippet}</ListRow>
        </div>
      </div>
    </Section>

    <!-- ======================== Feedback ======================== -->
    <Section id="feedback" title="Feedback" note="Banners, badges, spinners, empty states and separators.">
      <div class="stack">
        <Banner variant="info" title="Information">The vault is locked after 15 minutes of inactivity.</Banner>
        <Banner variant="warning" title="Certificate not trusted">The certificate is self-signed. Verify the fingerprint before continuing.</Banner>
        <Banner variant="danger" title="Host key changed">The server presented a different key than the one saved. Someone may be intercepting the connection.
          {#snippet actions()}<Button size="compact" variant="destructive">Replace key</Button><Button size="compact">Cancel</Button>{/snippet}
        </Banner>
        <Banner variant="success">Saved.</Banner>
        <Banner variant="info" ondismiss={() => (toolbarNote = "Banner dismissed")}>A dismissible banner.</Banner>
      </div>
      <h3 class="group">Badges</h3>
      <div class="row-wrap">
        {#each ["neutral", "accent", "danger", "warning", "success", "info"] as const as variant (variant)}
          <Badge {variant}>{variant}</Badge>
        {/each}
        <Badge>3</Badge>
      </div>
      <h3 class="group">Spinner and separators</h3>
      <div class="row-wrap">
        <Spinner size={14} /><Spinner size={16} /><Spinner size={20} label="Loading" />
        <div class="sep-demo"><span>Left</span><Separator orientation="vertical" /><span>Right</span></div>
      </div>
      <Separator />
      <Separator strong />
      <div class="empty-frame">
        <EmptyState icon="plug" title="No connections yet" text="Create your first connection to get started. Everything is stored encrypted on this computer.">
          {#snippet action()}<Button variant="primary">New connection</Button>{/snippet}
        </EmptyState>
        <EmptyState title="Portolano" text="Select a connection in the sidebar.">
          {#snippet graphic()}<Logo size={48} />{/snippet}
        </EmptyState>
      </div>
    </Section>

    <!-- ======================== Forms ======================== -->
    <Section id="forms" title="Forms" note="FormSection / FormRow lay out settings like macOS: right-aligned labels, optional grouped panel.">
      <div class="forms">
        <FormSection title="General" description="Basic properties of the connection.">
          <FormRow label="Name" for="g-name"><TextField id="g-name" aria-label="Name" value="prod-db-01" /></FormRow>
          <FormRow label="Protocol" for="g-proto"><Select id="g-proto" aria-label="Protocol" value="ssh" options={SELECT_OPTIONS} /></FormRow>
          <FormRow label="Host" for="g-host" hint="DNS name or IP address." error="Host is required."><TextField id="g-host" aria-label="Host" value="" error="" /></FormRow>
        </FormSection>
        <FormSection title="Grouped" grouped>
          <FormRow label="Auto-lock"><Switch checked aria-label="Auto-lock" /></FormRow>
          <FormRow label="Start in"><SegmentedControl label="Start in" value="a" options={[{ value: "a", label: "Window" }, { value: "b", label: "Tab" }]} /></FormRow>
          <FormRow label="Options"><Checkbox label="Remember window size" checked /></FormRow>
        </FormSection>
        <FormSection title="Stacked">
          <FormRow label="Description" layout="stacked" for="g-desc"><TextArea id="g-desc" aria-label="Description" value="" placeholder="Optional" /></FormRow>
        </FormSection>
      </div>
    </Section>

    <!-- ======================== Dialogs ======================== -->
    <Section id="dialogs" title="Dialogs and toasts" note="Open each one; Escape closes, Tab stays inside, focus returns to the button.">
      <div class="row-wrap">
        <Button onclick={() => (dialog = "sample")}>Sample dialog</Button>
        <Button onclick={() => (dialog = "large")}>Large dialog</Button>
        <Button onclick={() => (dialog = "hero")}>Dialog with hero strip</Button>
        <Button onclick={() => (dialog = "confirm")}>Confirm</Button>
        <Button onclick={() => (dialog = "danger")}>Confirm delete</Button>
        <Button onclick={() => (dialog = "prompt")}>Prompt</Button>
        <Button onclick={() => app.notices.push("info", "Connection saved", "prod-db-01 was updated.")}>Info toast</Button>
        <Button onclick={() => app.notices.push("warning", "Could not save the credential to the vault", "“prod-db-01” is connected, but its credential was not saved.")}>Warning toast</Button>
        <Button onclick={() => app.notices.push("error", "Could not connect", "Connection refused (connect_failed)")}>Error toast</Button>
        <Button onclick={() => (dialog = "import")}>Import preview</Button>
      </div>
    </Section>

    <!-- ======================== Window chrome ======================== -->
    <Section id="chrome" title="Window chrome" note="Toolbar with keyboard navigation, and the title-bar drag region. Switch the platform above to see the macOS inset (--titlebar-height).">
      <div class="frame">
        <div class="flush-demo">
          <Toolbar label="Flush toolbar (no padding)" flush>
            <IconButton icon="plus" label="New" size="compact" /><IconButton icon="folder-plus" label="New folder" size="compact" /><IconButton icon="more" label="More" size="compact" />
          </Toolbar>
          <span class="muted">flush: the parent supplies the gutter</span>
        </div>
        <Toolbar label="Sample toolbar" bordered>
          {#snippet start()}<IconButton icon="panel-left" label="Toggle sidebar" /><IconButton icon="arrow-left" label="Back" /><IconButton icon="arrow-right" label="Forward" disabled />{/snippet}
          <strong>Toolbar title</strong>
          {#snippet end()}<IconButton icon="search" label="Search" /><IconButton icon="plus" label="New" /><Button size="compact" variant="primary">Connect</Button>{/snippet}
        </Toolbar>
        <div class="titlebar-demo"><TitlebarDragRegion fill>
          <span class="muted">drag region ({platform === "macos" ? "40 px strip + 80 px safe area" : "0 px on Windows"})</span>
        </TitlebarDragRegion></div>
      </div>
    </Section>

    <!-- ======================== Sample window ======================== -->
    <Section id="sample" title="Sample window" note="Sidebar, tab strip and empty state composed from the parts above (static; not the real screens).">
      <div class="window">
        <aside class="win-sidebar">
          <TitlebarDragRegion />
          <div class="win-search"><TextField aria-label="Filter" placeholder="Filter" size="compact">{#snippet prefix()}<Icon name="search" size={14} />{/snippet}</TextField></div>
          <div class="win-list">
            <ListRow>{#snippet leading()}<Icon name="chevron-down" size={14} />{/snippet}<span class="inline"><KindIcon kind="folder" open /> Production</span></ListRow>
            <ListRow depth={1} selected>{#snippet leading()}<KindIcon kind="ssh" />{/snippet}prod-db-01{#snippet trailing()}<Badge variant="success">Live</Badge>{/snippet}</ListRow>
            <ListRow depth={1} forceHover>{#snippet leading()}<KindIcon kind="rdp" />{/snippet}Build server</ListRow>
            <ListRow depth={1} forceFocus>{#snippet leading()}<KindIcon kind="vnc" />{/snippet}Lab console</ListRow>
            <ListRow>{#snippet leading()}<Icon name="chevron-right" size={14} />{/snippet}<span class="inline"><KindIcon kind="folder" /> Home lab</span></ListRow>
            <ListRow>{#snippet leading()}<KindIcon kind="web" />{/snippet}Router admin</ListRow>
            <ListRow>{#snippet leading()}<KindIcon kind="localShell" />{/snippet}Local shell</ListRow>
          </div>
        </aside>
        <div class="win-main">
          <div class="win-tabs" role="tablist" aria-label="Sample tabs">
            <span class="win-tab active" role="tab" aria-selected="true" tabindex="0"><KindIcon kind="ssh" size={14} /> prod-db-01</span>
            <span class="win-tab" role="tab" aria-selected="false" tabindex="-1"><KindIcon kind="rdp" size={14} /> Build server</span>
          </div>
          <EmptyState icon="terminal" title="Session ended" text="The remote host closed the connection.">
            {#snippet action()}<Button variant="primary">Reconnect</Button>{/snippet}
          </EmptyState>
        </div>
      </div>

      <p class="note">Welcome screen: chart backdrop, hero and kind tiles (decoration never sits behind running text).</p>
      <div class="window">
        <aside class="win-sidebar">
          <div class="win-search"><TextField aria-label="Filter" placeholder="Filter" size="compact">{#snippet prefix()}<Icon name="search" size={14} />{/snippet}</TextField></div>
          <div class="win-list">
            <ListRow>{#snippet leading()}<Icon name="chevron-down" size={14} />{/snippet}<span class="inline"><KindIcon kind="folder" open size={20} /> Production</span></ListRow>
            <ListRow depth={1}><span class="inline"><KindIcon kind="ssh" size={20} /> prod-db-01</span></ListRow>
            <ListRow depth={1}><span class="inline"><KindIcon kind="rdp" size={20} /> Build server</span></ListRow>
            <ListRow><span class="inline"><KindIcon kind="web" size={20} /> Router admin</span></ListRow>
          </div>
          <div class="win-sidebar-foot"><ChartBackdrop placement="corner" labels={false} /></div>
        </aside>
        <div class="win-main win-welcome">
          <ChartBackdrop />
          <div class="hero">
            <span class="hero-logo"><Logo size={44} /></span>
            <h3 class="hero-title">Chart a course</h3>
            <p class="hero-text">Every host is a port of call. Pick a connection, or open a new one.</p>
            <div class="hero-tiles">
              <KindTile kind="ssh" label="SSH" hint="22" />
              <KindTile kind="rdp" label="RDP" hint="3389" />
              <KindTile kind="vnc" label="VNC" hint="5900" />
              <KindTile kind="web" label="Web" hint="443" />
              <KindTile kind="localShell" label="Shell" hint="local" />
            </div>
            <p class="hero-keys"><Kbd>⌘</Kbd><Kbd>N</Kbd> new connection <span class="hero-sep">·</span> <Kbd>⌘</Kbd><Kbd>K</Kbd> search</p>
          </div>
        </div>
      </div>
    </Section>
  </main>
</div>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={MENU_ITEMS} label="Sample menu" onclose={() => (menu = null)} />
{/if}

{#if dialog === "sample"}
  <Dialog title="Edit connection" onclose={() => (dialog = "none")}>
    <Banner variant="warning">This connection uses a password stored in the vault.</Banner>
    <div class="stack dialog-form">
      <TextField label="Name" value="prod-db-01" data-autofocus />
      <TextField label="Host" value="10.0.0.12" hint="DNS name or IP address." />
      <Select label="Protocol" value="ssh" options={SELECT_OPTIONS} />
      <Checkbox label="Connect on startup" />
    </div>
    {#snippet footer()}
      <Button onclick={() => (dialog = "none")}>Cancel</Button>
      <Button variant="primary" onclick={() => (dialog = "none")}>Save</Button>
    {/snippet}
  </Dialog>
{:else if dialog === "hero"}
  <Dialog title="Unlock vault" size="sm" onclose={() => (dialog = "none")}>
    {#snippet hero()}
      <div class="hero-strip">
        <ChartBackdrop labels={false} />
        <span class="hero-mark"><Icon name="lock" size={20} /></span>
      </div>
    {/snippet}
    <TextField label="Master password" type="password" data-autofocus />
    {#snippet footer()}
      <Button onclick={() => (dialog = "none")}>Cancel</Button>
      <Button variant="primary" onclick={() => (dialog = "none")}>Unlock</Button>
    {/snippet}
  </Dialog>
{:else if dialog === "large"}
  <Dialog title="Credentials" size="lg" onclose={() => (dialog = "none")}>
    {#each Array.from({ length: 24 }, (_, i) => i) as i (i)}
      <ListRow>Credential {i + 1}</ListRow>
    {/each}
    {#snippet footer()}<Button variant="primary" onclick={() => (dialog = "none")}>Done</Button>{/snippet}
  </Dialog>
{:else if dialog === "confirm"}
  <ConfirmDialog title="Lock vault?" message="Open sessions stay connected." danger={false} confirmLabel="Lock" onconfirm={() => (dialog = "none")} oncancel={() => (dialog = "none")} />
{:else if dialog === "danger"}
  <ConfirmDialog title="Delete “prod-db-01”?" message="This cannot be undone." error="The backend refused the request." onconfirm={() => (dialog = "none")} oncancel={() => (dialog = "none")} />
{:else if dialog === "import"}
  <ImportDialog path="/Users/demo/Sessions.mxtsessions" onclose={() => (dialog = "none")} />
{:else if dialog === "prompt"}
  <PromptDialog title="New folder" label="Name" initial="" submitLabel="Create" onsubmit={() => { dialog = "none"; return null; }} oncancel={() => (dialog = "none")} />
{/if}

<Toasts />

<style>
  .page {
    height: 100%;
    overflow: auto;
    background: var(--surface-content);
    color: var(--text-primary);
    -webkit-user-select: text;
    user-select: text;
  }
  .bar {
    position: sticky;
    top: 0;
    z-index: var(--z-sticky);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3) var(--space-4);
    padding: var(--space-2) var(--space-4);
    background: var(--surface-window);
    border-bottom: 1px solid var(--border-hairline);
  }
  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }
  nav {
    display: flex;
    flex: 1;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-3);
  }
  nav a {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
    text-decoration: none;
  }
  nav a:hover {
    color: var(--text-primary);
  }
  .switches {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .content {
    max-width: calc(var(--dialog-width-lg) * 1.8);
    margin: 0 auto;
    padding: var(--space-4) var(--space-4) var(--space-12);
  }
  .group {
    margin: var(--space-4) 0 var(--space-2);
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }
  .muted {
    color: var(--text-secondary);
  }
  code {
    font-size: var(--font-size-sm);
  }
  .swatches {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(calc(var(--tooltip-max-width) - var(--space-4)), 1fr));
    gap: var(--space-2);
  }
  .swatch {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .chip {
    flex: none;
    width: var(--space-8);
    height: var(--space-6);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
  }
  .mono-lines code {
    font-family: var(--font-mono);
    line-height: var(--terminal-line-height);
  }
  .letterbox {
    display: grid;
    place-items: center;
    padding: var(--space-6);
    background: var(--letterbox-bg);
    border-radius: var(--radius-panel);
  }
  .letterbox-desktop {
    display: grid;
    place-items: center;
    width: calc(var(--space-12) * 5);
    height: calc(var(--space-12) * 2);
    background: var(--surface-content);
    box-shadow: var(--shadow-popover);
    color: var(--text-secondary);
  }
  .ansi-panels {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(calc(var(--tooltip-max-width) * 1.5), 1fr));
    gap: var(--space-3);
  }
  .ansi-panel {
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-panel);
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
    line-height: var(--terminal-line-height);
  }
  .ansi-title {
    margin-bottom: var(--space-2);
    font-weight: var(--font-weight-semibold);
  }
  .ansi-row {
    display: flex;
    gap: var(--space-3);
  }
  .ansi-row code {
    font-family: inherit;
    font-size: inherit;
  }
  .ansi-sample {
    flex: 1;
  }
  .hero-strip {
    position: relative;
    display: grid;
    place-items: center;
    block-size: calc(var(--space-12) * 1.5);
  }
  .hero-mark {
    position: relative;
    display: grid;
    place-items: center;
    inline-size: calc(var(--space-4) * 2.5);
    block-size: calc(var(--space-4) * 2.5);
    background: var(--surface-raised);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
    box-shadow: var(--shadow-popover);
  }
  .flush-demo {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border-hairline);
  }
  .shadows {
    display: flex;
    gap: var(--space-6);
    padding: var(--space-4);
    background: var(--surface-window);
    border-radius: var(--radius-panel);
  }
  .shadow-box {
    display: grid;
    place-items: center;
    width: calc(var(--space-12) * 2);
    height: calc(var(--space-12) * 1.2);
    background: var(--surface-raised);
    border-radius: var(--radius-panel);
  }
  .type-row {
    margin-bottom: var(--space-2);
  }
  .spaces {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4);
    align-items: flex-end;
    margin-bottom: var(--space-4);
  }
  .space {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .bar-sample {
    height: var(--space-3);
    background: var(--text-tertiary);
    border-radius: 1px;
  }
  .shapes {
    display: flex;
    gap: var(--space-4);
    margin-bottom: var(--space-4);
  }
  .shape {
    display: grid;
    place-items: center;
    width: calc(var(--space-12) * 1.5);
    height: var(--space-12);
    background: var(--surface-window);
    border: 1px solid var(--border-strong);
    font-size: var(--font-size-sm);
  }
  .height {
    display: inline-flex;
    align-items: center;
    padding: 0 var(--space-3);
    background: var(--surface-window);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    font-size: var(--font-size-sm);
  }
  .row-wrap {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
  }
  .row-wrap.tight {
    gap: var(--space-2);
    margin-bottom: 0;
  }
  .row-label {
    width: calc(var(--space-12) * 1.6);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
  .icon-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(calc(var(--space-12) * 2), 1fr));
    gap: var(--space-3);
  }
  .icon-cell {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-1);
    color: var(--text-primary);
  }
  .icon-cell code {
    color: var(--text-secondary);
  }
  .grid2 {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(calc(var(--tooltip-max-width) + var(--space-8)), 1fr));
    gap: var(--space-4);
    margin-bottom: var(--space-4);
  }
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .dialog-form {
    margin-top: var(--space-3);
  }
  .menu-target {
    display: grid;
    place-items: center;
    width: calc(var(--space-12) * 3);
    height: calc(var(--space-12) * 1.5);
    background: var(--surface-window);
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-panel);
    color: var(--text-secondary);
  }
  .list-demo {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(calc(var(--tooltip-max-width) + var(--space-12)), 1fr));
    gap: var(--space-4);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: var(--space-2);
    background: var(--surface-sidebar);
    border-radius: var(--radius-panel);
  }
  .inline {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }
  .sep-demo {
    display: inline-flex;
    gap: var(--space-3);
    align-items: center;
    height: var(--control-h-regular);
  }
  .empty-frame {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(var(--toast-width), 1fr));
    margin-top: var(--space-4);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
  }
  .forms {
    max-width: calc(var(--dialog-width-lg) + var(--space-12));
  }
  .frame {
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
    overflow: hidden;
  }
  .titlebar-demo {
    display: flex;
    min-height: var(--space-12);
    background: var(--surface-sidebar);
    padding: var(--space-2);
  }
  .window {
    display: flex;
    height: calc(var(--space-12) * 8);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-panel);
    overflow: hidden;
    box-shadow: var(--shadow-popover);
  }
  .win-sidebar {
    display: flex;
    flex: none;
    flex-direction: column;
    width: calc(var(--space-12) * 5.5);
    background: var(--surface-sidebar);
    border-right: 1px solid var(--border-hairline);
  }
  .win-search {
    padding: var(--space-2);
  }
  .win-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 0 var(--space-2);
  }
  .win-main {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    background: var(--surface-content);
  }
  .win-tabs {
    display: flex;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2) 0;
    background: var(--surface-window);
    border-bottom: 1px solid var(--border-hairline);
  }
  .win-tab {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--control-h-regular);
    padding: 0 var(--space-3);
    border-radius: var(--radius-control) var(--radius-control) 0 0;
    color: var(--text-secondary);
  }
  .note {
    margin: var(--space-6) 0 var(--space-2);
    color: var(--text-secondary);
  }
  .win-sidebar-foot {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  .win-welcome {
    position: relative;
    align-items: center;
    justify-content: center;
  }
  .hero {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    text-align: center;
  }
  .hero-logo {
    display: inline-flex;
    color: var(--text-primary);
  }
  .hero-title {
    font-size: var(--font-size-hero);
    font-weight: var(--font-weight-semibold);
    line-height: var(--line-height-tight);
    color: var(--text-primary);
  }
  .hero-text {
    color: var(--text-secondary);
  }
  .hero-tiles {
    display: flex;
    gap: var(--space-2);
    margin: var(--space-4) 0 var(--space-3);
  }
  .hero-keys {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--text-tertiary);
    font-size: var(--font-size-sm);
  }
  .hero-sep {
    margin: 0 var(--space-1);
  }
  .win-tab.active {
    background: var(--surface-content);
    color: var(--text-primary);
  }
</style>
