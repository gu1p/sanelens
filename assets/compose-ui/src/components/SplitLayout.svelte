<script lang="ts">
  type RenderFn = () => unknown;

  let { sidebar, content } = $props<{
    sidebar?: RenderFn;
    content?: RenderFn;
  }>();
</script>

<div
  class="grid h-full min-h-0 grid-cols-1 grid-rows-[minmax(160px,35%)_minmax(0,1fr)] gap-6 px-4 pb-6 pt-4 sm:px-6 sm:pb-8 sm:pt-5 lg:grid-cols-[280px,1fr] lg:grid-rows-[minmax(0,1fr)] lg:px-8 lg:pb-12 lg:pt-6"
>
  <aside class="min-h-0">
    {@render sidebar?.()}
  </aside>
  <section class="min-h-0">
    {@render content?.()}
  </section>
</div>
