<script lang="ts">
  import { tick } from "svelte";
  import Icon from "./Icon.svelte";

  let {
    text,
    size = 14,
  }: {
    text: string;
    size?: number;
  } = $props();

  let open = $state(false);
  let ready = $state(false);
  let anchor = $state<HTMLElement | null>(null);
  let tip = $state<HTMLElement | null>(null);
  let left = $state(0);
  let top = $state(0);

  // Keep the tooltip inside the nearest clipping/scroll ancestor (dialogs, page content),
  // otherwise it would be cut off by that element's overflow.
  function bounds() {
    const inset = 6;
    let node = anchor?.parentElement ?? null;
    while (node && node !== document.body) {
      const style = getComputedStyle(node);
      if (/(auto|scroll|hidden|clip)/.test(style.overflowX + style.overflowY)) {
        const box = node.getBoundingClientRect();
        return {
          left: box.left + inset,
          top: box.top + inset,
          right: box.right - inset,
          bottom: box.bottom - inset,
        };
      }
      node = node.parentElement;
    }
    return {
      left: inset,
      top: inset,
      right: window.innerWidth - inset,
      bottom: window.innerHeight - inset,
    };
  }

  async function place() {
    const box = anchor?.getBoundingClientRect();
    const tipBox = tip?.getBoundingClientRect();
    if (!box || !tipBox) return;
    // `position: fixed` can be resolved against an ancestor that creates a containing block
    // (transformed / filtered / contained elements), so measure where the tip actually lands.
    const originX = tipBox.left - left;
    const originY = tipBox.top - top;
    const area = bounds();
    const gap = 8;

    const wantedLeft = box.left + box.width / 2 - tipBox.width / 2;
    const maxLeft = Math.max(area.left, area.right - tipBox.width);
    const wantedTop = box.top - tipBox.height - gap;

    left = Math.min(Math.max(wantedLeft, area.left), maxLeft) - originX;
    top = Math.min(Math.max(wantedTop, area.top), Math.max(area.top, area.bottom - tipBox.height)) - originY;
  }

  async function show() {
    open = true;
    ready = false;
    await tick();
    await place();
    ready = true;
  }

  async function follow() {
    if (!open) return;
    await place();
  }

  function hide() {
    open = false;
    ready = false;
  }

  $effect(() => {
    if (!open) return;
    const options = { capture: true, passive: true } as const;
    window.addEventListener("scroll", follow, options);
    window.addEventListener("resize", follow);
    return () => {
      window.removeEventListener("scroll", follow, options);
      window.removeEventListener("resize", follow);
    };
  });
</script>

<button
  type="button"
  class="hint"
  bind:this={anchor}
  aria-label={text}
  onclick={(event) => { event.preventDefault(); event.stopPropagation(); }}
  onmouseenter={show}
  onmouseleave={hide}
  onfocus={show}
  onblur={hide}
>
  <Icon name="alert" size={size} strokeWidth={1.6} />
</button>

{#if open}
  <div
    class="hint-tip"
    class:ready
    bind:this={tip}
    style:left={`${left}px`}
    style:top={`${top}px`}
  >{text}</div>
{/if}

<style>
  .hint {
    display: inline-flex; align-items: center; justify-content: center;
    vertical-align: middle; color: #b6bcc6; cursor: help; flex: 0 0 auto;
    padding: 0; margin: 0; border: 0; background: transparent; line-height: 0;
    border-radius: 50%; transition: color var(--dur-1) var(--ease);
  }
  .hint:hover { color: #7d8592; }
  .hint:focus-visible { color: #7d8592; outline-offset: 3px; }
  .hint-tip {
    position: fixed; z-index: 90; max-width: 264px; padding: 7px 10px;
    border: 1px solid var(--line); border-radius: 9px; background: rgba(255, 255, 255, .98);
    color: #5d6572; font-size: 11.5px; font-weight: 400; line-height: 1.55;
    letter-spacing: 0; text-align: left; white-space: normal; pointer-events: none;
    box-shadow: 0 10px 28px rgba(35, 44, 62, .12), 0 1px 3px rgba(35, 44, 62, .05);
    opacity: 0; transition: opacity var(--dur-1) var(--ease);
  }
  .hint-tip.ready { opacity: 1; }
</style>