<script lang="ts">
  import Button from "../ui/Button.svelte";
  import Chip from "../ui/Chip.svelte";

  type TabId = "logs" | "traffic";

  type TopBarProps = {
    onAddPanel?: () => void;
    activeTab?: TabId;
    onTabChange?: (tab: TabId) => void;
  };

  let {
    onAddPanel = () => {},
    activeTab = "logs",
    onTabChange = () => {},
  }: TopBarProps = $props();
</script>

<div
  class="flex flex-col gap-3 px-4 py-4 sm:flex-row sm:items-center sm:justify-between sm:px-6 sm:py-5 lg:px-8 lg:py-6"
>
  <div class="flex flex-col gap-1 uppercase tracking-[0.16em]">
    <div class="text-lg font-bold">Compose</div>
    <div class="text-xs text-muted">Log Deck</div>
  </div>
  <div class="flex items-center gap-3">
    <div class="flex items-center gap-1 rounded-full border border-ink/10 bg-panel/70 p-1">
      <Chip
        size="sm"
        active={activeTab === "logs"}
        muted={activeTab !== "logs"}
        onclick={() => onTabChange("logs")}
      >
        Logs
      </Chip>
      <Chip
        size="sm"
        active={activeTab === "traffic"}
        muted={activeTab !== "traffic"}
        onclick={() => onTabChange("traffic")}
      >
        Traffic
      </Chip>
    </div>
    {#if activeTab === "logs"}
      <Button variant="primary" onclick={onAddPanel}>Add panel</Button>
    {/if}
  </div>
</div>
