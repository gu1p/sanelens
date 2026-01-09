<script lang="ts">
  import Surface from "../ui/Surface.svelte";
  import ChipLink from "../ui/ChipLink.svelte";
  import { colorFor } from "../lib/colors";
  import { endpointLabel, getEndpoints } from "../lib/services";
  import type { ServiceInfo } from "../lib/types";

  let { services = [], error = null, onSelect = () => {} } = $props<{
    services?: ServiceInfo[];
    error?: string | null;
    onSelect?: (service: ServiceInfo) => void;
  }>();
</script>

<Surface class="h-full overflow-auto">
  <div class="text-xs font-semibold uppercase tracking-[0.25em]">Services</div>
  <p class="mt-2 text-xs text-muted">
    Click a service to focus the active panel. Use open to visit endpoints.
  </p>

  {#if error}
    <div class="mt-4 rounded-xl border border-accent/30 bg-[#fff3ed] p-3 text-sm text-accent">
      {error}
    </div>
  {:else}
    <div class="mt-4 flex flex-col gap-3">
      {#each services as service (service.name)}
        {@const endpoints = getEndpoints(service)}
        <div
          class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-2 rounded-xl border border-ink/10 bg-panel2 p-2"
        >
          <button
            type="button"
            class="flex items-center gap-2 text-left font-semibold text-ink"
            onclick={() => onSelect(service)}
          >
            <span
              class="h-2.5 w-2.5 rounded-full"
              style={`background: ${colorFor(service.name)};`}
            ></span>
            <span>{service.name}</span>
          </button>

          {#if endpoints.length}
            <div class="flex flex-col items-end gap-1">
              {#each endpoints as endpoint (endpoint)}
                <ChipLink href={endpoint} label={endpointLabel(endpoint)} />
              {/each}
            </div>
          {:else}
            <span class="text-[11px] uppercase text-muted">internal</span>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</Surface>
