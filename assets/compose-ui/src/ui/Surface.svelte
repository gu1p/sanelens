<script lang="ts">
  export type Tone = "panel" | "muted";
  type RenderFn = () => unknown;

  let {
    tag = "div",
    tone = "panel",
    padded = true,
    class: className = "",
    children,
    ...rest
  } = $props<{
    tag?: keyof HTMLElementTagNameMap;
    tone?: Tone;
    padded?: boolean;
    class?: string;
    children?: RenderFn;
  }>();

  const base = "rounded-2xl border border-ink/10 shadow-panel";
  const tones: Record<Tone, string> = {
    panel: "bg-panel",
    muted: "bg-panel2",
  };
</script>

<svelte:element
  this={tag}
  class={`${base} ${tones[tone]} ${padded ? "p-4" : ""} ${className}`}
  {...rest}
>
  {@render children?.()}
</svelte:element>
