<script lang="ts">
  import Button from "../ui/Button.svelte";

  let { onAddPanel = () => {} } = $props();
</script>

<div
  class="flex flex-col gap-3 px-4 py-4 sm:flex-row sm:items-center sm:justify-between sm:px-6 sm:py-5 lg:px-8 lg:py-6"
>
  <div class="flex flex-col gap-1 uppercase tracking-[0.16em]">
    <div class="text-lg font-bold">Compose</div>
    <div class="text-xs text-muted">Log Deck</div>
  </div>
  <div class="flex items-center gap-3">
    <Button variant="primary" onclick={onAddPanel}>Add panel</Button>
  </div>
</div>
