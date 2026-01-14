<script lang="ts">
  import Chip from "../ui/Chip.svelte";
  import Surface from "../ui/Surface.svelte";
  import TextInput from "../ui/TextInput.svelte";
  import TrafficPanel from "./TrafficPanel.svelte";
  import type { EntityId, TrafficCall, TrafficEdge } from "../lib/types";

  type StatusFilter = "all" | "2xx" | "3xx" | "4xx" | "5xx" | "error";

  type TrafficExplorerProps = {
    calls?: TrafficCall[];
    edges?: TrafficEdge[];
    edgeError?: string | null;
    callError?: string | null;
  };

  let { calls = [], edges = [], edgeError = null, callError = null }: TrafficExplorerProps =
    $props();

  let search = $state("");
  let statusFilter: StatusFilter = $state("all");
  let pinnedCallId: number | null = $state(null);

  const statusOptions: { label: string; value: StatusFilter }[] = [
    { label: "All", value: "all" },
    { label: "2xx", value: "2xx" },
    { label: "3xx", value: "3xx" },
    { label: "4xx", value: "4xx" },
    { label: "5xx", value: "5xx" },
    { label: "Errors", value: "error" },
  ];

  const timeFormatter = new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });

  function entityLabel(entity?: EntityId | null) {
    if (!entity) {
      return "unknown";
    }
    switch (entity.kind) {
      case "workload":
        return entity.name;
      case "external":
        return entity.dns_name ?? entity.ip;
      case "host":
        return entity.name;
      default:
        return "unknown";
    }
  }

  function formatTime(value?: number | null) {
    if (!value) {
      return "--";
    }
    return timeFormatter.format(new Date(value));
  }

  function formatLatency(value?: number | null) {
    if (value === null || value === undefined) {
      return "--";
    }
    if (value >= 1000) {
      return `${(value / 1000).toFixed(2)}s`;
    }
    return `${value}ms`;
  }

  function formatBytes(value?: number | null) {
    if (value === null || value === undefined) {
      return "--";
    }
    const units = ["B", "KB", "MB", "GB"];
    let idx = 0;
    let size = value;
    while (size >= 1024 && idx < units.length - 1) {
      size /= 1024;
      idx += 1;
    }
    const precision = size < 10 && idx > 0 ? 1 : 0;
    return `${size.toFixed(precision)}${units[idx]}`;
  }

  function statusLabel(status?: number | null) {
    if (status === null || status === undefined) {
      return "--";
    }
    return String(status);
  }

  function statusTone(status?: number | null) {
    if (status === null || status === undefined) {
      return "text-muted";
    }
    if (status >= 500) {
      return "text-accent";
    }
    if (status >= 400) {
      return "text-accent";
    }
    if (status >= 300) {
      return "text-accent2";
    }
    return "text-accent3";
  }

  function statusMatches(call: TrafficCall) {
    const status = call.status ?? null;
    if (statusFilter === "all") {
      return true;
    }
    if (status === null) {
      return statusFilter === "error";
    }
    if (statusFilter === "error") {
      return status >= 400;
    }
    if (statusFilter === "2xx") {
      return status >= 200 && status < 300;
    }
    if (statusFilter === "3xx") {
      return status >= 300 && status < 400;
    }
    if (statusFilter === "4xx") {
      return status >= 400 && status < 500;
    }
    return status >= 500 && status < 600;
  }

  function callSearchTarget(call: TrafficCall) {
    const host = call.request_headers?.host ?? "";
    const requestId = call.correlation?.request_id ?? "";
    return [
      call.method ?? "",
      call.path ?? "",
      host,
      requestId,
      entityLabel(call.peer?.src),
      entityLabel(call.peer?.dst),
      statusLabel(call.status),
    ]
      .join(" ")
      .toLowerCase();
  }

  const filteredCalls = $derived.by(() => {
    const query = search.trim().toLowerCase();
    return calls.filter((call) => {
      if (!statusMatches(call)) {
        return false;
      }
      if (!query) {
        return true;
      }
      return callSearchTarget(call).includes(query);
    });
  });

  const selectedCallId = $derived.by(() => {
    if (!filteredCalls.length) {
      return null;
    }
    if (pinnedCallId !== null && filteredCalls.some((call) => call.seq === pinnedCallId)) {
      return pinnedCallId;
    }
    return filteredCalls[0].seq;
  });

  const selectedCall = $derived.by(() => {
    if (selectedCallId === null) {
      return null;
    }
    return filteredCalls.find((call) => call.seq === selectedCallId) ?? null;
  });

  const requestContentType = $derived.by(
    () => selectedCall?.request_headers?.["content-type"] ?? null,
  );
  const responseContentType = $derived.by(
    () => selectedCall?.response_headers?.["content-type"] ?? null,
  );
</script>

<div class="flex h-full min-h-0 flex-col gap-4">
  <div
    class="rounded-3xl border border-ink/10 bg-panel/70 p-4 shadow-[var(--shadow)] sm:p-5"
  >
    <div class="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
      <div>
        <div class="text-xs uppercase tracking-[0.12em] text-muted">Traffic</div>
        <div class="text-lg font-semibold">Request explorer</div>
      </div>
      <div class="text-xs text-muted">{calls.length} calls captured</div>
    </div>

    <div class="mt-3 flex flex-col gap-3 sm:flex-row sm:items-center sm:gap-4">
      <div class="w-full sm:max-w-sm">
        <TextInput
          value={search}
          placeholder="Search by method, path, host, service, or id"
          ariaLabel="Search traffic"
          onInput={(value) => (search = value)}
        />
      </div>
      <div class="flex flex-wrap items-center gap-2">
        {#each statusOptions as option (option.value)}
          <Chip
            size="xs"
            active={statusFilter === option.value}
            muted={statusFilter !== option.value}
            onclick={() => (statusFilter = option.value)}
          >
            {option.label}
          </Chip>
        {/each}
      </div>
    </div>
  </div>

  <div class="grid min-h-0 flex-1 gap-4 lg:grid-cols-[minmax(0,1.1fr)_minmax(0,0.9fr)]">
    <Surface class="flex min-h-0 flex-col gap-3">
      <div class="flex items-center justify-between">
        <div class="text-xs font-semibold uppercase tracking-[0.2em] text-muted">Calls</div>
        <div class="text-xs text-muted">{filteredCalls.length} shown</div>
      </div>

      {#if callError}
        <div class="rounded-xl border border-accent/30 bg-[#fff3ed] p-3 text-sm text-accent">
          {callError}
        </div>
      {:else if filteredCalls.length === 0}
        <div class="rounded-xl border border-ink/10 bg-panel2 p-3 text-sm text-muted">
          No calls match the current filters.
        </div>
      {:else}
        <div class="min-h-0 flex-1 overflow-auto rounded-2xl border border-ink/10 bg-panel2">
          <div class="divide-y divide-ink/10">
            {#each filteredCalls as call (call.seq)}
              <button
                type="button"
                class={`flex w-full flex-col gap-2 px-4 py-3 text-left transition hover:bg-[#fff6ea] ${
                  call.seq === selectedCallId ? "bg-[#fff1df]" : ""
                }`}
                onclick={() => (pinnedCallId = call.seq)}
              >
                <div class="flex items-center justify-between gap-3">
                  <div class="min-w-0">
                    <div class="flex flex-wrap items-center gap-2">
                      <span class="text-[11px] font-semibold uppercase text-muted">
                        {formatTime(call.at_ms)}
                      </span>
                      <span class="rounded-full border border-ink/10 bg-panel px-2 py-0.5 text-[11px] font-semibold text-ink">
                        {(call.method ?? "UNKNOWN").toUpperCase()}
                      </span>
                      <span class="truncate text-sm font-semibold">
                        {call.path ?? "(no path)"}
                      </span>
                    </div>
                    <div class="mt-1 flex flex-wrap items-center gap-3 text-[11px] text-muted">
                      <span>{entityLabel(call.peer?.src)} -> {entityLabel(call.peer?.dst)}</span>
                      <span>{formatLatency(call.duration_ms)}</span>
                      <span>{formatBytes(call.bytes_in)} in</span>
                      <span>{formatBytes(call.bytes_out)} out</span>
                    </div>
                  </div>
                  <span class={`text-sm font-semibold ${statusTone(call.status)}`}>
                    {statusLabel(call.status)}
                  </span>
                </div>
              </button>
            {/each}
          </div>
        </div>
      {/if}
    </Surface>

    <div class="flex min-h-0 flex-col gap-4">
      <TrafficPanel edges={edges} error={edgeError} />

      <Surface class="flex min-h-0 flex-col gap-3">
        <div class="flex items-center justify-between">
          <div class="text-xs font-semibold uppercase tracking-[0.2em] text-muted">Inspector</div>
          <div class="text-xs text-muted">
            {selectedCall ? formatTime(selectedCall.at_ms) : "Select a call"}
          </div>
        </div>

        {#if !selectedCall}
          <div class="flex min-h-0 flex-1 items-center justify-center text-sm text-muted">
            Select a call to inspect headers and payloads.
          </div>
        {:else}
          <div class="min-h-0 flex-1 space-y-3 overflow-auto pr-1">
            <div class="rounded-xl border border-ink/10 bg-panel2 p-3">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="text-sm font-semibold">
                  {(selectedCall.method ?? "UNKNOWN").toUpperCase()} {selectedCall.path ?? ""}
                </div>
                <div class={`text-sm font-semibold ${statusTone(selectedCall.status)}`}>
                  {statusLabel(selectedCall.status)}
                </div>
              </div>
              <div class="mt-2 flex flex-wrap items-center gap-3 text-[11px] text-muted">
                <span>{entityLabel(selectedCall.peer?.src)} -> {entityLabel(selectedCall.peer?.dst)}</span>
                <span>{formatLatency(selectedCall.duration_ms)}</span>
                <span>{formatBytes(selectedCall.bytes_in)} in</span>
                <span>{formatBytes(selectedCall.bytes_out)} out</span>
              </div>
              {#if selectedCall.correlation?.request_id}
                <div class="mt-2 text-[11px] text-muted">
                  request id: {selectedCall.correlation.request_id}
                </div>
              {/if}
            </div>

            <div class="grid gap-3 lg:grid-cols-2">
              <div class="rounded-xl border border-ink/10 bg-panel2 p-3">
                <div class="text-[11px] font-semibold uppercase tracking-[0.2em] text-muted">
                  Request
                </div>
                {#if Object.keys(selectedCall.request_headers ?? {}).length === 0}
                  <div class="mt-2 text-xs text-muted">No headers captured.</div>
                {:else}
                  <div class="mt-2 grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)] gap-2 text-[11px]">
                    {#each Object.entries(selectedCall.request_headers ?? {}) as [key, value] (key)}
                      <div class="truncate text-muted">{key}</div>
                      <div class="break-words font-mono text-ink">{value}</div>
                    {/each}
                  </div>
                {/if}
                {#if requestContentType}
                  <div class="mt-2 text-[11px] text-muted">
                    content-type: {requestContentType}
                  </div>
                {/if}
                {#if selectedCall.request_body}
                  <div class="mt-2 max-h-48 overflow-auto rounded-lg border border-ink/10 bg-[#fff8ef] p-2 font-mono text-[11px] text-ink/80">
                    <pre class="whitespace-pre-wrap">{selectedCall.request_body}</pre>
                  </div>
                {:else}
                  <div class="mt-2 text-xs text-muted">
                    No body captured. Size: {formatBytes(selectedCall.bytes_in)}
                  </div>
                {/if}
              </div>

              <div class="rounded-xl border border-ink/10 bg-panel2 p-3">
                <div class="text-[11px] font-semibold uppercase tracking-[0.2em] text-muted">
                  Response
                </div>
                {#if Object.keys(selectedCall.response_headers ?? {}).length === 0}
                  <div class="mt-2 text-xs text-muted">No headers captured.</div>
                {:else}
                  <div class="mt-2 grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)] gap-2 text-[11px]">
                    {#each Object.entries(selectedCall.response_headers ?? {}) as [key, value] (key)}
                      <div class="truncate text-muted">{key}</div>
                      <div class="break-words font-mono text-ink">{value}</div>
                    {/each}
                  </div>
                {/if}
                {#if responseContentType}
                  <div class="mt-2 text-[11px] text-muted">
                    content-type: {responseContentType}
                  </div>
                {/if}
                {#if selectedCall.response_body}
                  <div class="mt-2 max-h-48 overflow-auto rounded-lg border border-ink/10 bg-[#fff8ef] p-2 font-mono text-[11px] text-ink/80">
                    <pre class="whitespace-pre-wrap">{selectedCall.response_body}</pre>
                  </div>
                {:else}
                  <div class="mt-2 text-xs text-muted">
                    No body captured. Size: {formatBytes(selectedCall.bytes_out)}
                  </div>
                {/if}
              </div>
            </div>
          </div>
        {/if}
      </Surface>
    </div>
  </div>
</div>
