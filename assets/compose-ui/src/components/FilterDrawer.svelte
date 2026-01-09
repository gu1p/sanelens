<script lang="ts">
  import { tick } from "svelte";
  import Chip from "../ui/Chip.svelte";
  import FilterRow from "./FilterRow.svelte";
  import { normalizeFilterToken } from "../lib/filters";
  import type { PanelState } from "../lib/types";

  type FilterType = "include" | "exclude";

  let { open = false, panel = null, meta = "", onClose = () => {}, onUpdate = () => {} } = $props<{
    open?: boolean;
    panel?: PanelState | null;
    meta?: string;
    onClose?: () => void;
    onUpdate?: (include: string[], exclude: string[]) => void;
  }>();

  let includeDraft = $state<string[]>([""]);
  let excludeDraft = $state<string[]>([""]);
  let includeListEl: HTMLDivElement | null = null;
  let excludeListEl: HTMLDivElement | null = null;
  let lastPanelId: string | null = null;

  $effect(() => {
    const nextId = panel?.id ?? null;
    if (nextId === lastPanelId) {
      return;
    }
    lastPanelId = nextId;
    includeDraft = panel?.include?.length ? [...panel.include] : [""];
    excludeDraft = panel?.exclude?.length ? [...panel.exclude] : [""];
  });

  function normalizeDraft(values: string[]): string[] {
    return values.map((value) => normalizeFilterToken(value)).filter(Boolean);
  }

  function updateFilters() {
    if (!panel) {
      return;
    }
    onUpdate(normalizeDraft(includeDraft), normalizeDraft(excludeDraft));
  }

  async function addRow(type: FilterType) {
    if (type === "include") {
      includeDraft.push("");
    } else {
      excludeDraft.push("");
    }
    await tick();
    focusLastInput(type);
  }

  function removeRow(type: FilterType, index: number) {
    const target = type === "include" ? includeDraft : excludeDraft;
    target.splice(index, 1);
    if (target.length === 0) {
      target.push("");
    }
    updateFilters();
  }

  function updateValue(type: FilterType, index: number, value: string) {
    const target = type === "include" ? includeDraft : excludeDraft;
    target[index] = value;
    updateFilters();
  }

  async function clearFilters() {
    includeDraft = [""];
    excludeDraft = [""];
    updateFilters();
    await tick();
    focusFirstInput();
  }

  function focusLastInput(type: FilterType) {
    const listEl = type === "include" ? includeListEl : excludeListEl;
    const inputs = listEl?.querySelectorAll("input");
    const last = inputs?.[inputs.length - 1] as HTMLInputElement | undefined;
    last?.focus();
  }

  function focusFirstInput() {
    const input = includeListEl?.querySelector("input") as HTMLInputElement | null;
    input?.focus();
  }

  function handleKeydown(event: KeyboardEvent) {
    if (!open) {
      return;
    }
    if (event.key === "Escape") {
      onClose();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<aside
  class={`fixed bottom-6 right-6 top-24 z-30 ${open ? "block" : "hidden"}`}
  aria-hidden={!open}
>
  <div
    class="flex h-full w-[min(var(--drawer-width),92vw)] flex-col gap-4 overflow-hidden rounded-2xl border border-ink/10 bg-panel p-4 shadow-panel"
    role="dialog"
    aria-labelledby="filter-drawer-title"
  >
    <div class="flex items-start justify-between gap-3">
      <div class="flex flex-col gap-1">
        <div class="text-base font-semibold" id="filter-drawer-title">
          {panel ? `${panel.title} filters` : "Panel filters"}
        </div>
        <div class="text-[11px] uppercase tracking-[0.16em] text-muted">{meta}</div>
      </div>
      <Chip size="sm" ghost onclick={onClose}>Close</Chip>
    </div>

    <div class="grid flex-1 grid-cols-[repeat(auto-fit,minmax(200px,1fr))] gap-3 overflow-auto pr-1">
      <div class="flex flex-col gap-2">
        <div class="flex items-center justify-between gap-3">
          <span class="text-[11px] font-semibold uppercase tracking-[0.2em] text-muted">Include</span>
          <Chip size="xs" ghost onclick={() => addRow("include")}>Add</Chip>
        </div>
        <div class="flex max-h-48 flex-col gap-2 overflow-y-auto pr-1" bind:this={includeListEl}>
          {#each includeDraft as value, index (index)}
            <FilterRow
              {value}
              placeholder="error"
              tone="include"
              onInput={(next) => updateValue("include", index, next)}
              onRemove={() => removeRow("include", index)}
              onEnter={() => addRow("include")}
            />
          {/each}
        </div>
      </div>

      <div class="flex flex-col gap-2">
        <div class="flex items-center justify-between gap-3">
          <span class="text-[11px] font-semibold uppercase tracking-[0.2em] text-muted">Exclude</span>
          <Chip size="xs" ghost onclick={() => addRow("exclude")}>Add</Chip>
        </div>
        <div class="flex max-h-48 flex-col gap-2 overflow-y-auto pr-1" bind:this={excludeListEl}>
          {#each excludeDraft as value, index (index)}
            <FilterRow
              {value}
              placeholder="healthcheck"
              tone="exclude"
              onInput={(next) => updateValue("exclude", index, next)}
              onRemove={() => removeRow("exclude", index)}
              onEnter={() => addRow("exclude")}
            />
          {/each}
        </div>
      </div>
    </div>

    <div class="mt-auto flex justify-end gap-2">
      <Chip size="sm" onclick={clearFilters}>Clear</Chip>
      <Chip size="sm" active onclick={onClose}>Done</Chip>
    </div>
  </div>
</aside>
