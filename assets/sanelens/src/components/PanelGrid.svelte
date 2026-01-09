<script lang="ts">
  import PanelCard from "./PanelCard.svelte";
  import type { PanelState, ServiceInfo } from "../lib/types";

  let {
    panels = [],
    services = [],
    activePanelId = null,
    onActivate = () => {},
    onToggleFollow = () => {},
    onOpenFilters = () => {},
    onClose = () => {},
    onToggleService = () => {},
    onSelectAll = () => {},
  } = $props<{
    panels?: PanelState[];
    services?: ServiceInfo[];
    activePanelId?: string | null;
    onActivate?: (id: string) => void;
    onToggleFollow?: (panel: PanelState) => void;
    onOpenFilters?: (panel: PanelState) => void;
    onClose?: (panel: PanelState) => void;
    onToggleService?: (panel: PanelState, name: string) => void;
    onSelectAll?: (panel: PanelState) => void;
  }>();
</script>

<div
  class="grid h-full min-h-0 auto-rows-[minmax(0,1fr)] grid-cols-[repeat(auto-fit,minmax(320px,1fr))] items-start gap-4 overflow-hidden"
>
  {#each panels as panel (panel.id)}
    <PanelCard
      {panel}
      {services}
      isActive={panel.id === activePanelId}
      {onActivate}
      {onToggleFollow}
      {onOpenFilters}
      {onClose}
      {onToggleService}
      {onSelectAll}
    />
  {/each}
</div>
