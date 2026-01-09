<script lang="ts">
  import type { ButtonHTMLAttributes } from "svelte/elements";

  export type ChipSize = "xs" | "sm";

  type RenderFn = () => unknown;

  let {
    active = false,
    muted = false,
    ghost = false,
    size = "sm",
    type = "button",
    class: className = "",
    children,
    ...rest
  } = $props<{
    active?: boolean;
    muted?: boolean;
    ghost?: boolean;
    size?: ChipSize;
    type?: ButtonHTMLAttributes["type"];
    class?: string;
    children?: RenderFn;
  }>();

  const base =
    "inline-flex items-center gap-1 rounded-full border text-xs font-semibold transition duration-150 hover:-translate-y-0.5";
  const sizes: Record<ChipSize, string> = {
    xs: "px-2 py-0.5 text-[11px]",
    sm: "px-3 py-1",
  };
</script>

<button
  type={type}
  class={`${base} ${sizes[size]} ${ghost ? "border-dashed bg-transparent" : "bg-[#fff9f1]"} ${
    muted ? "text-muted" : "text-ink"
  } ${active ? "border-transparent text-[#0f1512] chip-active" : "border-ink/15"} ${className}`}
  {...rest}
>
  {@render children?.()}
</button>
