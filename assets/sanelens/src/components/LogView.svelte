<script lang="ts">
  import { colorFor } from "../lib/colors";
  import type { LogEvent } from "../lib/types";

  let { logs = [], autoScroll = true } = $props<{
    logs?: LogEvent[];
    autoScroll?: boolean;
  }>();

  let container: HTMLDivElement | null = null;
  let lastCount = 0;

  $effect(() => {
    if (!container) {
      return;
    }
    if (autoScroll && logs.length !== lastCount) {
      container.scrollTop = container.scrollHeight;
    }
    lastCount = logs.length;
  });
</script>

<div
  class="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto rounded-xl bg-[#171411] p-3 font-mono text-xs text-[#f6f1ea] shadow-[inset_0_0_0_1px_rgba(255,255,255,0.06)]"
  bind:this={container}
>
  {#each logs as entry (entry.seq)}
    <div
      class="grid grid-cols-[auto_auto_1fr] items-start gap-2 py-0.5 text-[12px] leading-relaxed animate-fadeIn"
    >
      <span
        class="text-[10px] font-semibold uppercase tracking-[0.14em]"
        style={`color: ${colorFor(entry.service)};`}
      >
        {entry.service}
      </span>
      <span class="text-[10px] text-white/60">{entry.container_ts ?? ""}</span>
      <span class="whitespace-pre-wrap break-words">{entry.line}</span>
    </div>
  {/each}
</div>
