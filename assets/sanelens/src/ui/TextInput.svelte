<script lang="ts">
  export type InputTone = "include" | "exclude" | "neutral";

  let {
    value = "",
    placeholder = "",
    ariaLabel = "",
    tone = "neutral",
    class: className = "",
    onInput,
    onEnter,
    ...rest
  } = $props<{
    value?: string;
    placeholder?: string;
    ariaLabel?: string;
    tone?: InputTone;
    class?: string;
    onInput?: (value: string) => void;
    onEnter?: () => void;
  }>();

  const accents: Record<InputTone, string> = {
    include: "var(--accent-3)",
    exclude: "var(--accent)",
    neutral: "var(--accent-2)",
  };
</script>

<input
  class={`w-full rounded-xl border border-ink/15 border-l-[3px] bg-[#fffdf8] px-3 py-2 text-xs font-mono text-ink transition focus:outline-none focus:ring-2 focus:ring-ink/20 ${className}`}
  style={`border-left-color: ${accents[tone]};`}
  value={value}
  {placeholder}
  aria-label={ariaLabel}
  autocomplete="off"
  autocapitalize="off"
  spellcheck="false"
  oninput={(event) => onInput?.((event.currentTarget as HTMLInputElement).value)}
  onkeydown={(event) => {
    if (event.key === "Enter" && onEnter) {
      event.preventDefault();
      onEnter();
    }
  }}
  {...rest}
/>
