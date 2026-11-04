<script lang="ts">
  let {
    name,
    size = 18,
    strokeWidth = 1.7,
  }: {
    name:
      | "grid"
      | "clock"
      | "bolt"
      | "history"
      | "code"
      | "sliders"
      | "search"
      | "plus"
      | "play"
      | "stop"
      | "edit"
      | "trash"
      | "close"
      | "refresh"
      | "folder"
      | "file"
      | "download"
      | "panel"
      | "alert";
    size?: number;
    strokeWidth?: number;
  } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === "grid"}
    <rect x="3.5" y="3.5" width="7" height="7" rx="2"></rect>
    <rect x="13.5" y="3.5" width="7" height="7" rx="2"></rect>
    <rect x="3.5" y="13.5" width="7" height="7" rx="2"></rect>
    <rect x="13.5" y="13.5" width="7" height="7" rx="2"></rect>
  {:else if name === "clock"}
    <circle cx="12" cy="12" r="8.5"></circle>
    <path d="M12 7.5V12l3 2"></path>
  {:else if name === "bolt"}
    <path d="M13.2 2.8 5.4 13h5.7l-.8 8.2L18 10.6h-5.8l1-7.8Z"></path>
  {:else if name === "history"}
    <path d="M4.2 9.2A8.5 8.5 0 1 1 4 14"></path>
    <path d="M4 4.5v5h5"></path>
    <path d="M12 7.8V12l2.8 1.8"></path>
  {:else if name === "code"}
    <path d="m8.5 7.5-5 4.5 5 4.5"></path>
    <path d="m15.5 7.5 5 4.5-5 4.5"></path>
    <path d="m13.5 4.5-3 15"></path>
  {:else if name === "sliders"}
    <path d="M4 6h7"></path><path d="M15 6h5"></path>
    <path d="M4 12h3"></path><path d="M11 12h9"></path>
    <path d="M4 18h9"></path><path d="M17 18h3"></path>
    <circle cx="13" cy="6" r="2"></circle>
    <circle cx="9" cy="12" r="2"></circle>
    <circle cx="15" cy="18" r="2"></circle>
  {:else if name === "search"}
    <circle cx="10.8" cy="10.8" r="6.6"></circle>
    <path d="m16 16 4.2 4.2"></path>
  {:else if name === "plus"}
    <path d="M12 5v14M5 12h14"></path>
  {:else if name === "play"}
    <path d="M8 5.5v13l10-6.5-10-6.5Z"></path>
  {:else if name === "stop"}
    <rect x="7" y="7" width="10" height="10" rx="2"></rect>
  {:else if name === "edit"}
    <path d="m4.5 19.5 4.1-1 10.2-10.2a2.2 2.2 0 0 0-3.1-3.1L5.5 15.4l-1 4.1Z"></path>
    <path d="m14.5 6.5 3 3"></path>
  {:else if name === "trash"}
    <path d="M4.5 7h15"></path>
    <path d="M9 7V4.8h6V7"></path>
    <path d="m6.5 7 .8 12.2h9.4L17.5 7"></path>
    <path d="M10 11v4.8M14 11v4.8"></path>
  {:else if name === "close"}
    <path d="m6.5 6.5 11 11M17.5 6.5l-11 11"></path>
  {:else if name === "refresh"}
    <path d="M20 11a8 8 0 1 0-2.3 6.2"></path>
    <path d="M20 4.5V11h-6.5"></path>
  {:else if name === "folder"}
    <path d="M3.5 7.5h6l2 2h9v9.2a1.8 1.8 0 0 1-1.8 1.8H5.3a1.8 1.8 0 0 1-1.8-1.8V7.5Z"></path>
    <path d="M3.5 7.5V5.8A1.8 1.8 0 0 1 5.3 4h4.2l2 2h7.2a1.8 1.8 0 0 1 1.8 1.8v1.7"></path>
  {:else if name === "file"}
    <path d="M6 3.5h7.5l4.5 4.5v12.5H6V3.5Z"></path>
    <path d="M13.5 3.5V8H18"></path>
    <path d="M9 12.5h6M9 16h4.5"></path>
  {:else if name === "download"}
    <path d="M12 4v10.5"></path>
    <path d="m7.8 10.4 4.2 4.2 4.2-4.2"></path>
    <path d="M5 18.5h14"></path>
  {:else if name === "panel"}
    <rect x="3.5" y="4.5" width="17" height="15" rx="3"></rect>
    <path d="M9.5 4.5v15"></path>
  {:else if name === "alert"}
    <circle cx="12" cy="12" r="8.6"></circle>
    <path d="M12 7.6v5.2"></path>
    <path d="M12 16.4h.01"></path>
  {/if}
</svg>
