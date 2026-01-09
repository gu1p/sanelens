<script lang="ts">
  import Chip from "../ui/Chip.svelte";
  import TextInput from "../ui/TextInput.svelte";
  import type { InputTone } from "../ui/TextInput.svelte";

  let {
    value = "",
    placeholder = "",
    tone = "neutral",
    onInput = () => {},
    onRemove = () => {},
    onEnter = () => {},
  } = $props<{
    value?: string;
    placeholder?: string;
    tone?: InputTone;
    onInput?: (value: string) => void;
    onRemove?: () => void;
    onEnter?: () => void;
  }>();
</script>

<div class="grid grid-cols-[1fr_auto] items-center gap-2">
  <TextInput
    {value}
    {placeholder}
    ariaLabel={placeholder}
    {tone}
    onInput={onInput}
    onEnter={onEnter}
  />
  <Chip size="xs" ghost onclick={onRemove} aria-label="Remove filter">
    x
  </Chip>
</div>
