<script lang="ts">
  import { confirmRequest } from "./confirm";

  let confirmButton: HTMLButtonElement | undefined = $state();

  function answer(ok: boolean) {
    const request = $confirmRequest;
    if (!request) return;
    confirmRequest.set(null);
    request.resolve(ok);
  }

  function onKey(event: KeyboardEvent) {
    if ($confirmRequest && event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      answer(false);
    }
  }

  $effect(() => {
    if ($confirmRequest) confirmButton?.focus();
  });
</script>

<svelte:window onkeydowncapture={onKey} />

{#if $confirmRequest}
  <div class="modal-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) answer(false); }}>
    <div class="modal confirm" role="alertdialog" aria-modal="true" aria-label={$confirmRequest.title}>
      <div class="modal-head"><div class="modal-title">{$confirmRequest.title}</div></div>
      <div class="modal-body"><p class="confirm-message">{$confirmRequest.message}</p></div>
      <div class="modal-foot">
        <button class="btn" onclick={() => answer(false)}>取消</button>
        <button
          bind:this={confirmButton}
          class="btn"
          class:btn-danger={$confirmRequest.danger}
          class:btn-primary={!$confirmRequest.danger}
          onclick={() => answer(true)}
        >{$confirmRequest.confirmLabel}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .confirm { width: min(400px, 100%); }
  .confirm-message { margin: 0; font-size: 13px; line-height: 1.65; color: #4a525c; white-space: pre-line; }
</style>
