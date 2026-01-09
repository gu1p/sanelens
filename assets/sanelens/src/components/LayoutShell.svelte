<script lang="ts">
  type RenderFn = () => unknown;

  let { drawerOpen = false, header, children } = $props<{
    drawerOpen?: boolean;
    header?: RenderFn;
    children?: RenderFn;
  }>();
</script>

<div class="flex min-h-screen flex-col overflow-hidden">
  <header
    class="sticky top-0 z-20 border-b border-ink/10 bg-[rgba(245,239,230,0.9)] backdrop-blur-md"
  >
    {@render header?.()}
  </header>
  <main
    class={`flex-1 min-h-0 overflow-hidden ${
      drawerOpen ? "mr-[calc(min(var(--drawer-width),92vw)+24px)]" : ""
    }`}
  >
    {@render children?.()}
  </main>
</div>
