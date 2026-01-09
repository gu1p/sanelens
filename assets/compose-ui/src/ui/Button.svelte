<script lang="ts">
  import type { ButtonHTMLAttributes } from "svelte/elements";

  export type ButtonVariant = "primary" | "surface" | "ghost";
  export type ButtonSize = "sm" | "md";

  type RenderFn = () => unknown;

  let {
    variant = "surface",
    size = "md",
    type = "button",
    class: className = "",
    children,
    ...rest
  } = $props<{
    variant?: ButtonVariant;
    size?: ButtonSize;
    type?: ButtonHTMLAttributes["type"];
    class?: string;
    children?: RenderFn;
  }>();

  const base =
    "inline-flex items-center justify-center rounded-full border text-sm font-semibold transition duration-150";
  const variants: Record<ButtonVariant, string> = {
    primary: "bg-accent text-[#fffaf3] border-transparent shadow-sm hover:shadow-lg",
    surface: "bg-panel text-ink border-border hover:-translate-y-0.5 hover:shadow-lg",
    ghost: "bg-transparent text-ink border-ink/30 hover:-translate-y-0.5",
  };
  const sizes: Record<ButtonSize, string> = {
    sm: "px-3 py-1 text-xs",
    md: "px-4 py-2",
  };
</script>

<button
  type={type}
  class={`${base} ${variants[variant]} ${sizes[size]} ${className}`}
  {...rest}
>
  {@render children?.()}
</button>
