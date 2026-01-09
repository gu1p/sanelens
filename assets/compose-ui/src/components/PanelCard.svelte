<script lang="ts">
  import Chip from "../ui/Chip.svelte";
  import ChipLink from "../ui/ChipLink.svelte";
  import Surface from "../ui/Surface.svelte";
  import LogView from "./LogView.svelte";
  import { buildPanelMeta } from "../lib/filters";
  import { colorFor } from "../lib/colors";
  import { endpointLabel, getEndpoints } from "../lib/services";
  import type { PanelState, ServiceInfo } from "../lib/types";

  let {
    panel,
    services = [],
    isActive = false,
    onActivate = () => {},
    onToggleFollow = () => {},
    onOpenFilters = () => {},
    onClose = () => {},
    onToggleService = () => {},
    onSelectAll = () => {},
  } = $props<{
    panel: PanelState;
    services?: ServiceInfo[];
    isActive?: boolean;
    onActivate?: (id: string) => void;
    onToggleFollow?: (panel: PanelState) => void;
    onOpenFilters?: (panel: PanelState) => void;
    onClose?: (panel: PanelState) => void;
    onToggleService?: (panel: PanelState, name: string) => void;
    onSelectAll?: (panel: PanelState) => void;
  }>();

  const meta = $derived.by(() => buildPanelMeta(panel));
</script>

<Surface
  tag="article"
  class={`flex h-full min-h-[360px] flex-col overflow-hidden animate-liftIn ${
    isActive ? "outline outline-2 outline-accent3 outline-offset-2" : ""
  }`}
  style={`animation-delay: ${panel.delay}s;`}
  onmousedown={() => onActivate(panel.id)}
>
  <header class="flex items-start justify-between gap-3">
    <div class="flex flex-col gap-1">
      <div class="text-base font-semibold">{panel.title}</div>
      <div class="text-[11px] uppercase tracking-[0.18em] text-muted">{meta}</div>
    </div>
    <div class="flex items-center gap-2">
      <Chip
        size="sm"
        onmousedown={(event) => event.stopPropagation()}
        onclick={() => onOpenFilters(panel)}
      >
        Filters
      </Chip>
      <Chip
        size="sm"
        active={panel.autoScroll}
        muted={!panel.autoScroll}
        onmousedown={(event) => event.stopPropagation()}
        onclick={() => onToggleFollow(panel)}
      >
        {panel.autoScroll ? "Follow" : "Paused"}
      </Chip>
      <Chip
        size="sm"
        ghost
        onmousedown={(event) => event.stopPropagation()}
        onclick={() => onClose(panel)}
      >
        Close
      </Chip>
    </div>
  </header>

  <div class="my-3 flex flex-wrap gap-2">
    <Chip
      size="sm"
      active={panel.filter === null}
      style="--chip-color: #f2cc8f;"
      onclick={() => onSelectAll(panel)}
    >
      All
    </Chip>
    {#each services as service (service.name)}
      {@const endpoints = getEndpoints(service)}
      <div class="flex items-center gap-2">
        <Chip
          size="sm"
          active={panel.filter ? panel.filter.includes(service.name) : false}
          style={`--chip-color: ${colorFor(service.name)};`}
          onclick={() => onToggleService(panel, service.name)}
        >
          {service.name}
        </Chip>
        {#each endpoints as endpoint (endpoint)}
          <ChipLink href={endpoint} label={endpointLabel(endpoint)} />
        {/each}
      </div>
    {/each}
  </div>

  <LogView logs={panel.logs} autoScroll={panel.autoScroll} />
</Surface>
