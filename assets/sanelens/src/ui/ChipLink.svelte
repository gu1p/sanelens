<script lang="ts">
  type RenderFn = () => unknown;

  let { href = "", label = "", class: className = "", children, ...rest } = $props<{
    href?: string;
    label?: string;
    class?: string;
    children?: RenderFn;
  }>();
</script>

<a
  href={href}
  target="_blank"
  rel="noreferrer"
  class={`inline-flex items-center rounded-full border border-dashed border-ink/20 px-2 py-0.5 text-[11px] font-semibold text-accent2 transition hover:border-solid ${className}`}
  {...rest}
>
  {#if children}
    {@render children()}
  {:else}
    {label}
  {/if}
</a>
