<script lang="ts">
  import { tick } from "svelte";

  import { t, type StringKey } from "../i18n/strings";
  import type { Node, Uuid } from "../ipc";
  import { detectMac, shortcutLabel } from "../shortcuts";
  import { getApp } from "../state/app";
  import Badge from "../ui/Badge.svelte";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import ChartBackdrop from "../ui/ChartBackdrop.svelte";
  import ConfirmDialog from "../ui/ConfirmDialog.svelte";
  import ContextMenu from "../ui/ContextMenu.svelte";
  import EmptyState from "../ui/EmptyState.svelte";
  import Icon from "../ui/Icon.svelte";
  import IconButton from "../ui/IconButton.svelte";
  import KindIcon from "../ui/KindIcon.svelte";
  import ListRow from "../ui/ListRow.svelte";
  import type { MenuItem } from "../ui/menu";
  import { startExport, startImport } from "../transfer/actions";
  import PromptDialog from "../ui/PromptDialog.svelte";
  import Spinner from "../ui/Spinner.svelte";
  import TextField from "../ui/TextField.svelte";
  import MoveDialog from "./MoveDialog.svelte";
  import SidebarHeader from "./SidebarHeader.svelte";
  import {
    buildTree,
    checkMove,
    defaultParentFor,
    descendantCount,
    filterNodes,
    flattenTree,
    isConnection,
    jumpHostDependents,
    nodeKind,
    subtreeIds,
    type TreeRow,
  } from "./model";

  const app = getApp();
  const { doc, ui, tabs, notices } = app;

  let query = $state("");
  let searchInput: HTMLInputElement | undefined = $state();
  const isMac = detectMac();
  let treeEl: HTMLDivElement | undefined = $state();

  const filter = $derived(filterNodes(doc.nodes, query));
  const roots = $derived(buildTree(doc.nodes));
  const rows = $derived(flattenTree(roots, doc.expanded, filter));
  const focusId = $derived(
    ui.selectedId !== null && rows.some((r) => r.node.id === ui.selectedId)
      ? ui.selectedId
      : (rows[0]?.node.id ?? null),
  );

  type Pending =
    | { type: "rename"; node: Node }
    | { type: "delete"; node: Node; error: string | null }
    | { type: "move"; node: Node }
    | { type: "newFolder"; parentId: Uuid | null };
  let pending = $state<Pending | null>(null);
  let menu = $state<{ x: number; y: number; node: Node | null; kind?: "more" } | null>(null);
  let draggingId = $state<Uuid | null>(null);
  let dropTarget = $state<Uuid | "root" | null>(null);

  $effect(() => {
    if (ui.searchFocusRequest > 0) {
      searchInput?.focus();
      searchInput?.select();
    }
  });

  function clearSearch(): void {
    query = "";
    searchInput?.focus();
  }

  function newConnectionHere(): void {
    ui.open({
      type: "connection",
      id: null,
      parentId: defaultParentFor(doc.nodes, ui.selectedId),
    });
  }

  function newFolderHere(): void {
    pending = { type: "newFolder", parentId: defaultParentFor(doc.nodes, ui.selectedId) };
  }

  function openMoreMenu(event: MouseEvent): void {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { x: rect.left, y: rect.bottom, node: null, kind: "more" };
  }

  async function focusRow(id: Uuid): Promise<void> {
    ui.selectedId = id;
    await tick();
    treeEl?.querySelector<HTMLElement>(`[data-node-id="${CSS.escape(id)}"]`)?.focus();
  }

  function open(node: Node): void {
    if (isConnection(node)) void tabs.openConnection(node.id);
    else doc.setExpanded(node.id, !doc.isExpanded(node.id));
  }

  // ---- mutations ----

  async function rename(node: Node, name: string): Promise<string | null> {
    const result = await doc.upsert({ ...node, name });
    if (result.ok) {
      pending = null;
      return null;
    }
    return result.error.message;
  }

  async function createFolder(parentId: Uuid | null, name: string): Promise<string | null> {
    const result = await doc.upsert({
      type: "folder",
      id: crypto.randomUUID(),
      parentId,
      name,
      credential: { type: "inherit" },
    });
    if (!result.ok) return result.error.message;
    if (parentId !== null) doc.expand(parentId);
    pending = null;
    await focusRow(result.value.id);
    return null;
  }

  async function confirmDelete(node: Node): Promise<void> {
    const doomed = subtreeIds(doc.nodes, node.id);
    const result = await doc.remove(node.id);
    if (!result.ok) {
      pending = { type: "delete", node, error: `${result.error.message} (${result.error.code})` };
      return;
    }
    pending = null;
    await tabs.closeForConnections(doomed);
    if (ui.selectedId !== null && doomed.has(ui.selectedId)) ui.selectedId = null;
  }

  /** What deleting `target` does, including the SSH connections that lose their jump host. */
  function deleteMessage(target: Node): string {
    const folder = target.type === "folder";
    const count = folder ? descendantCount(doc.nodes, target.id) : 0;
    const base = folder
      ? count === 0
        ? t("tree.deleteFolderEmpty", { name: target.name })
        : t("tree.deleteFolderBody", { name: target.name, count })
      : t("tree.deleteConnectionBody", { name: target.name });
    const dependents = jumpHostDependents(doc.nodes, subtreeIds(doc.nodes, target.id)).length;
    if (dependents === 0) return base;
    const key: StringKey = folder
      ? dependents === 1
        ? "tree.deleteJumpFolder.one"
        : "tree.deleteJumpFolder.other"
      : dependents === 1
        ? "tree.deleteJumpConn.one"
        : "tree.deleteJumpConn.other";
    return `${base}\n\n${t(key, { count: dependents })}`;
  }

  async function moveNode(id: Uuid, parentId: Uuid | null): Promise<string | null> {
    const check = checkMove(doc.nodes, id, parentId);
    if (!check.ok) return check.reason === "unchanged" ? null : t("tree.moveInvalid");
    const result = await doc.move(id, parentId);
    if (result.ok) return null;
    return `${result.error.message} (${result.error.code})`;
  }

  async function moveFromDialog(node: Node, parentId: Uuid | null): Promise<string | null> {
    const error = await moveNode(node.id, parentId);
    if (error === null) pending = null;
    return error;
  }

  // ---- menus ----

  function nodeMenu(node: Node): MenuItem[] {
    const items: MenuItem[] = [];
    if (isConnection(node)) {
      items.push({
        label: t("tree.open"),
        action: () => open(node),
        icon: "plug",
        shortcut: isMac ? "↵" : "Enter",
      });
    } else {
      items.push(
        {
          label: t("tree.newConnectionHere"),
          action: () => ui.open({ type: "connection", id: null, parentId: node.id }),
          icon: "plus",
        },
        {
          label: t("tree.newFolderHere"),
          action: () => (pending = { type: "newFolder", parentId: node.id }),
          icon: "folder-plus",
        },
      );
    }
    items.push(
      {
        label: t("tree.edit"),
        action: () => ui.open({ type: node.type, id: node.id, parentId: node.parentId }),
        separatorBefore: true,
        icon: "settings",
      },
      {
        label: t("tree.rename"),
        action: () => (pending = { type: "rename", node }),
        icon: "edit",
        shortcut: "F2",
      },
      {
        label: t("tree.moveTo"),
        action: () => (pending = { type: "move", node }),
        icon: "arrow-right",
      },
      {
        label: t("transfer.exportNode"),
        action: () =>
          void startExport(app, { format: "json", nodeIds: [node.id], name: node.name }),
        icon: "upload",
        separatorBefore: true,
      },
      ...(isConnection(node) && node.options.type === "rdp"
        ? [
            {
              label: t("transfer.exportRdp"),
              action: () =>
                void startExport(app, { format: "rdp", nodeIds: [node.id], name: node.name }),
              icon: "upload" as const,
            },
          ]
        : []),
      {
        label: t("tree.delete"),
        action: () => (pending = { type: "delete", node, error: null }),
        danger: true,
        separatorBefore: true,
        icon: "trash",
        shortcut: isMac ? "⌦" : "Del",
      },
    );
    return items;
  }

  function blankMenu(): MenuItem[] {
    return [
      {
        label: t("tree.newConnection"),
        action: () => ui.open({ type: "connection", id: null, parentId: null }),
        icon: "plus",
        shortcut: shortcutLabel("newConnection", isMac),
      },
      {
        label: t("tree.newFolder"),
        action: () => (pending = { type: "newFolder", parentId: null }),
        icon: "folder-plus",
      },
    ];
  }

  function moreMenu(): MenuItem[] {
    return [
      { label: t("tree.collapseAll"), action: () => doc.collapseAll(), icon: "chevrons-up-down" },
      { label: t("tree.expandAll"), action: () => doc.expandAll(), icon: "chevrons-down-up" },
      ...blankMenu().map((item, index) => ({ ...item, separatorBefore: index === 0 })),
      {
        label: t("transfer.import"),
        action: () => void startImport(app),
        icon: "download",
        separatorBefore: true,
      },
      {
        label: t("transfer.exportAll"),
        action: () => void startExport(app, { format: "json", nodeIds: null, name: "portolano-connections" }),
        icon: "upload",
      },
      {
        label: t("update.menu"),
        action: () => ui.open({ type: "updates" }),
        icon: "refresh",
        separatorBefore: true,
      },
    ];
  }

  function openMenuAt(event: MouseEvent, node: Node | null): void {
    event.preventDefault();
    event.stopPropagation();
    if (node) ui.selectedId = node.id;
    menu = { x: event.clientX, y: event.clientY, node };
  }

  function openMenuForRow(row: TreeRow): void {
    const el = treeEl?.querySelector<HTMLElement>(`[data-node-id="${CSS.escape(row.node.id)}"]`);
    const rect = el?.getBoundingClientRect();
    ui.selectedId = row.node.id;
    menu = { x: rect?.left ?? 0, y: rect ? rect.bottom : 0, node: row.node };
  }

  // ---- keyboard ----

  function onTreeKeydown(event: KeyboardEvent): void {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const target = event.target instanceof Element ? event.target.closest("[data-node-id]") : null;
    const currentId = target?.getAttribute("data-node-id") ?? focusId;
    const index = rows.findIndex((r) => r.node.id === currentId);
    const row = rows[index];
    if (!row) return;
    const go = (target: TreeRow | undefined): void => {
      if (target) void focusRow(target.node.id);
    };
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        go(rows[index + 1]);
        break;
      case "ArrowUp":
        event.preventDefault();
        go(rows[index - 1]);
        break;
      case "Home":
        event.preventDefault();
        go(rows[0]);
        break;
      case "End":
        event.preventDefault();
        go(rows[rows.length - 1]);
        break;
      case "ArrowRight":
        event.preventDefault();
        if (row.expandable && !row.expanded) doc.setExpanded(row.node.id, true);
        else if (row.expanded) go(rows[index + 1]?.parentId === row.node.id ? rows[index + 1] : undefined);
        break;
      case "ArrowLeft":
        event.preventDefault();
        if (row.expanded && !filter) doc.setExpanded(row.node.id, false);
        else go(rows.find((r) => r.node.id === row.parentId));
        break;
      case "Enter":
        event.preventDefault();
        open(row.node);
        break;
      case "F2":
        event.preventDefault();
        pending = { type: "rename", node: row.node };
        break;
      case "Delete":
        event.preventDefault();
        pending = { type: "delete", node: row.node, error: null };
        break;
      case "ContextMenu":
        event.preventDefault();
        openMenuForRow(row);
        break;
      case "F10":
        if (event.shiftKey) {
          event.preventDefault();
          openMenuForRow(row);
        }
        break;
    }
  }

  function onSearchKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      query = "";
    } else if (event.key === "ArrowDown" || event.key === "Enter") {
      const first = rows[0];
      if (first) {
        event.preventDefault();
        void focusRow(first.node.id);
      }
    }
  }

  // ---- drag and drop ----

  function canDropOn(target: Uuid | null): boolean {
    return draggingId !== null && checkMove(doc.nodes, draggingId, target).ok;
  }

  function onDragStart(event: DragEvent, node: Node): void {
    draggingId = node.id;
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
      // Only the node id (no secrets) travels in the drag payload.
      event.dataTransfer.setData("text/plain", node.id);
    }
  }

  function onDragEnd(): void {
    draggingId = null;
    dropTarget = null;
  }

  function onRowDragOver(event: DragEvent, row: TreeRow): void {
    if (row.node.type !== "folder") return; // bubbles to the root drop zone logic below
    event.stopPropagation();
    if (canDropOn(row.node.id)) {
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
      dropTarget = row.node.id;
    } else {
      if (event.dataTransfer) event.dataTransfer.dropEffect = "none";
      dropTarget = null;
    }
  }

  function onRootDragOver(event: DragEvent): void {
    if (canDropOn(null)) {
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
      dropTarget = "root";
    }
  }

  async function dropOn(event: DragEvent, target: Uuid | null): Promise<void> {
    event.preventDefault();
    event.stopPropagation();
    const id = draggingId;
    onDragEnd();
    if (id === null) return;
    const error = await moveNode(id, target);
    if (error) notices.push("error", t("tree.opFailed"), error);
  }

  function describeRow(row: TreeRow): string {
    const n = row.node;
    return isConnection(n) ? `${n.name} ${n.host}` : n.name;
  }
</script>

<section class="tree-panel" aria-label={t("tree.label")}>
  <SidebarHeader
    onnewconnection={newConnectionHere}
    onnewfolder={newFolderHere}
    onmore={openMoreMenu}
  />

  <div class="search">
    <TextField
      bind:value={query}
      bind:input={searchInput}
      type="search"
      placeholder={t("tree.search")}
      aria-label={t("tree.searchLabel")}
      onkeydown={onSearchKeydown}
    >
      {#snippet prefix()}<Icon name="search" size={14} />{/snippet}
      {#snippet suffix()}
        {#if query !== ""}
          <IconButton
            icon="close"
            size="compact"
            label={t("tree.clearSearch")}
            tooltip=""
            onclick={clearSearch}
          />
        {/if}
      {/snippet}
    </TextField>
  </div>

  <div class="body">
    <ChartBackdrop placement="corner" labels={false} />
    {#if doc.status === "loading"}
      <div class="state" role="status">
        <Spinner size={14} />
        <span>{t("app.loading")}</span>
      </div>
    {:else if doc.status === "error"}
      <div class="state column">
        <Banner variant="danger">
          {t("app.loadFailed")}: {doc.loadError?.message} ({doc.loadError?.code})
        </Banner>
        <Button onclick={() => doc.load()}>{t("app.retry")}</Button>
      </div>
    {:else}
      {#if rows.length === 0}
        <div class="above">
          {#if filter}
            <EmptyState icon="search" title={t("tree.noMatchesTitle")} text={t("tree.noMatches")} />
          {:else}
            <EmptyState icon="plug" title={t("tree.emptyTitle")} text={t("tree.emptyHint")}>
              {#snippet action()}
                <div class="empty-actions">
                  <Button variant="primary" onclick={newConnectionHere}>
                    {#snippet icon()}<Icon name="plus" />{/snippet}
                    {t("tree.newConnection")}
                  </Button>
                  <Button onclick={() => void startImport(app)}>
                    {#snippet icon()}<Icon name="download" />{/snippet}
                    {t("tree.importHere")}
                  </Button>
                </div>
              {/snippet}
            </EmptyState>
          {/if}
        </div>
      {/if}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        bind:this={treeEl}
        class="tree"
        class:drop-root={dropTarget === "root"}
        role="tree"
        aria-label={t("tree.label")}
        tabindex="-1"
        onkeydown={onTreeKeydown}
        oncontextmenu={(e) => openMenuAt(e, null)}
        ondragover={onRootDragOver}
        ondrop={(e) => dropOn(e, null)}
        ondragleave={() => (dropTarget = null)}
      >
        {#each rows as row (row.node.id)}
          {@const node = row.node}
          <ListRow
            role="treeitem"
            depth={row.depth}
            selected={ui.selectedId === node.id}
            density="comfortable"
            dropTarget={dropTarget === node.id}
            dragging={draggingId === node.id}
            aria-level={row.depth + 1}
            aria-posinset={row.posInSet}
            aria-setsize={row.setSize}
            aria-expanded={row.expandable ? row.expanded : undefined}
            aria-selected={ui.selectedId === node.id}
            aria-label={describeRow(row)}
            tabindex={focusId === node.id ? 0 : -1}
            data-node-id={node.id}
            data-kind={nodeKind(node)}
            draggable="true"
            onclick={() => (ui.selectedId = node.id)}
            onfocus={() => (ui.selectedId = node.id)}
            ondblclick={() => open(node)}
            oncontextmenu={(e) => openMenuAt(e, node)}
            ondragstart={(e) => onDragStart(e, node)}
            ondragend={onDragEnd}
            ondragover={(e) => onRowDragOver(e, row)}
            ondrop={(e) => (node.type === "folder" ? dropOn(e, node.id) : undefined)}
          >
            {#snippet leading()}
              <span class="lead">
                {#if row.expandable}
                  <button
                    type="button"
                    class="twisty"
                    class:open={row.expanded}
                    tabindex="-1"
                    aria-hidden="true"
                    onclick={(e) => {
                      e.stopPropagation();
                      if (!filter) doc.setExpanded(node.id, !row.expanded);
                    }}><Icon name="chevron-right" size={14} /></button
                  >
                {:else}
                  <span class="twisty spacer"></span>
                {/if}
                <KindIcon kind={nodeKind(node)} open={row.expanded} size={20} />
              </span>
            {/snippet}
            <span class="name" title={node.name}>{node.name}</span>
            {#snippet secondary()}
              {#if isConnection(node)}<span class="host" title={node.host}>{node.host}</span>{/if}
            {/snippet}
            {#snippet trailing()}
              {#if isConnection(node) && node.tags.length > 0}
                <span class="tags" title={t("tree.tagsTitle", { tags: node.tags.join(", ") })}>
                  <Badge>{node.tags[0]}</Badge>
                  {#if node.tags.length > 1}<Badge>+{node.tags.length - 1}</Badge>{/if}
                </span>
              {/if}
            {/snippet}
          </ListRow>
        {/each}
      </div>
    {/if}
  </div>
</section>

{#if menu}
  <ContextMenu
    x={menu.x}
    y={menu.y}
    label={t("tree.label")}
    items={menu.kind === "more" ? moreMenu() : menu.node ? nodeMenu(menu.node) : blankMenu()}
    onclose={() => (menu = null)}
  />
{/if}

{#if pending?.type === "rename"}
  {@const target = pending.node}
  <PromptDialog
    title={t("tree.renameTitle")}
    label={t("tree.renameLabel")}
    initial={target.name}
    onsubmit={(name) => rename(target, name)}
    oncancel={() => (pending = null)}
  />
{:else if pending?.type === "newFolder"}
  {@const parentId = pending.parentId}
  <PromptDialog
    title={t("tree.newFolderTitle")}
    label={t("tree.newFolderName")}
    initial={t("tree.newFolderDefault")}
    submitLabel={t("common.create")}
    onsubmit={(name) => createFolder(parentId, name)}
    oncancel={() => (pending = null)}
  />
{:else if pending?.type === "delete"}
  {@const target = pending.node}
  <ConfirmDialog
    title={target.type === "folder" ? t("tree.deleteFolderTitle") : t("tree.deleteConnectionTitle")}
    message={deleteMessage(target)}
    error={pending.error}
    onconfirm={() => confirmDelete(target)}
    oncancel={() => (pending = null)}
  />
{:else if pending?.type === "move"}
  {@const target = pending.node}
  <MoveDialog
    node={target}
    nodes={doc.nodes}
    onmove={(parentId) => moveFromDialog(target, parentId)}
    oncancel={() => (pending = null)}
  />
{/if}

<style>
  .tree-panel {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }

  .search {
    flex: none;
    padding: var(--space-1) var(--space-2) var(--space-2);
  }

  /* Holds the tree and the corner chart; the chart fills whatever the rows leave free. */
  .body {
    position: relative;
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }

  /* The rows sit one hairline apart and are inset from the panel edges. */
  .tree {
    position: relative;
    display: grid;
    /* Without an explicit track the rows grow as wide as their longest name and run under
       the sidebar edge; minmax(0, 1fr) lets ListRow truncate them with an ellipsis. */
    grid-template-columns: minmax(0, 1fr);
    flex: 1;
    align-content: start;
    gap: 1px;
    min-height: var(--control-h-large);
    padding: 0 var(--space-2) var(--space-2);
    overflow: auto;
    border-radius: var(--radius-control);
  }

  .tree.drop-root {
    background: var(--accent-subtle);
    outline: 2px dashed var(--accent);
    outline-offset: calc(-1 * var(--space-1));
  }

  .lead {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    user-select: none;
  }

  .twisty {
    display: inline-grid;
    flex: none;
    place-items: center;
    width: var(--icon-size-md);
    height: var(--icon-size-md);
    padding: 0;
    background: none;
    border: 0;
    border-radius: var(--radius-xs);
    color: var(--text-tertiary);
    transition: rotate var(--duration-fast) var(--ease-standard);
  }

  .twisty.open {
    rotate: 90deg;
  }

  .twisty:hover {
    color: var(--text-primary);
  }

  .name {
    font-weight: var(--font-weight-regular);
  }

  .host {
    font-size: var(--font-size-sm);
  }

  .tags {
    display: inline-flex;
    gap: var(--space-1);
    max-width: calc(var(--space-12) * 2);
    overflow: hidden;
  }

  .above {
    position: relative;
    flex: none;
  }

  .state {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    color: var(--text-secondary);
  }

  .empty-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    justify-content: center;
  }

  .state.column {
    flex-direction: column;
    align-items: stretch;
  }
</style>
