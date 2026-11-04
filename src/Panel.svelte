<script lang="ts">
  import { onMount } from "svelte";
  import { api, friendlyError } from "./lib/api";
  import { isActiveStatus, isBusyStatus } from "./lib/format";
  import { createListenerGroup } from "./lib/listeners";
  import Icon from "./lib/Icon.svelte";
  import type { RunStateEvent, RuntimeMeta, Script } from "./lib/types";
  import { t } from "./lib/i18n.svelte";

  let scripts: Script[] = $state([]);
  let catalog: RuntimeMeta[] = $state([]);
  let loading = $state(true);
  let error = $state("");
  let busy = $state<Record<string, boolean>>({});

  const runningCount = $derived(scripts.filter((s) => isActiveStatus(s.status)).length);
  const runtimeNames = $derived(new Map(catalog.map((item) => [item.id, item.name] as const)));

  function runtimeLabel(id: string): string {
    return runtimeNames.get(id) ?? id;
  }

  function isBusy(script: Script): boolean {
    return busy[script.id] === true || isBusyStatus(script.status);
  }

  async function refresh() {
    try {
      scripts = await api.listScripts();
      error = "";
    } catch (e) {
      error = friendlyError(e);
    } finally {
      loading = false;
    }
  }

  async function adoptExternalRuns() {
    try {
      const count = await api.adoptExternalRuns();
      if (count > 0) await refresh();
    } catch (e) {
      error = friendlyError(e);
    }
  }

  async function toggle(script: Script) {
    if (isBusy(script) || !script.enabled) return;
    busy[script.id] = true;
    try {
      if (script.status === "running") {
        await api.stopScript(script.id);
      } else {
        await api.runScript(script.id);
      }
    } catch (e) {
      error = friendlyError(e);
    } finally {
      busy[script.id] = false;
    }
  }

  async function stopAll() {
    try {
      await api.stopAllScripts();
    } catch (e) {
      error = friendlyError(e);
    }
  }

  async function closePanel() {
    try { await api.closePanel(); } catch (e) { console.error(e); }
  }

  async function openMain() {
    try { await api.showMain(); } catch (e) { error = friendlyError(e); }
  }

  onMount(() => {
    refresh().finally(adoptExternalRuns);
    api.listRuntimeCatalog().then((items) => { catalog = items; }).catch(() => {});
    const listeners = createListenerGroup();
    listeners.add<RunStateEvent>("run-state", (event) => {
      const payload = event.payload;
      scripts = scripts.map((script) => script.id === payload.script_id
        ? { ...script, status: payload.status, last_error: payload.error, last_run_id: payload.run_id }
        : script);
    });
    listeners.add("scripts-changed", refresh);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") closePanel();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      listeners.dispose();
      window.removeEventListener("keydown", onKey);
    };
  });
</script>

<div class="panel-shell">
  <div class="panel-card">
    <header class="panel-head">
      <div class="panel-title">
        <span class="logo-mark">LS</span>
        <span>LiteScriptManager</span>
      </div>
      <div class="panel-head-actions">
        <span class="count-badge">{scripts.length}</span>
        <button class="close-btn" title={t("关闭 (Esc)")} aria-label={t("关闭")} onclick={closePanel}><Icon name="close" size={16} /></button>
      </div>
    </header>

    <div class="panel-subhead">
      <span>{runningCount > 0 ? t("{count} 个运行中", { count: runningCount }) : t("已配置脚本")}</span>
      <span class="live-hint"><span class="pulse"></span>{t("实时状态")}</span>
    </div>

    <main class="panel-list">
      {#if loading}
        <div class="panel-loading"><span class="spinner"></span>{t("加载中…")}</div>
      {:else if scripts.length === 0}
        <div class="panel-empty">
          <div class="empty-mark"><Icon name="grid" size={28} strokeWidth={1.3} /></div>
          <div>{t("还没有脚本")}</div>
          <button class="panel-link" onclick={openMain}>{t("打开管理添加 →")}</button>
        </div>
      {:else}
        {#each scripts as script (script.id)}
          <div class="panel-row" class:disabled={!script.enabled}>
            <span class="status-dot {script.enabled ? script.status : 'disabled'}"></span>
            <div class="row-info">
              <div class="row-name" title={script.name}>{script.name}</div>
              <div class="row-meta">
                <span>{runtimeLabel(script.runtime_id)}</span>
                {#if script.last_error}<span class="row-error" title={script.last_error}>{t("上次失败")}</span>{/if}
              </div>
            </div>
            <button
              class="toggle-btn"
              class:running={script.status === "running"}
              disabled={!script.enabled || isBusy(script)}
              title={script.status === "running" ? t("停止脚本") : t("启动脚本")}
              onclick={() => toggle(script)}
            >
              {#if isBusy(script)}<span class="btn-spinner"></span>
              {:else if script.status === "running"}<Icon name="stop" size={13} />
              {:else}<Icon name="play" size={13} />{/if}
            </button>
          </div>
        {/each}
      {/if}
    </main>

    {#if error}
      <div class="panel-error">{error}</div>
    {/if}

    <footer class="panel-foot">
      <button class="foot-btn" onclick={stopAll}>{t("全部停止")}</button>
      <button class="foot-btn primary" onclick={openMain}>{t("打开管理")}</button>
    </footer>
  </div>
</div>

<style>
  :global(html), :global(body) { height: 100%; background: transparent; }
  .panel-shell { height: 100vh; display: flex; }
  .panel-card {
    width: 100%; height: 100%; display: flex; flex-direction: column; overflow: hidden;
    background: #fff; border-radius: 14px;
  }
  .panel-head { display: flex; align-items: center; justify-content: space-between; padding: 15px 16px 9px; }
  .panel-title { display: flex; align-items: center; gap: 9px; font-size: 13px; font-weight: 600; letter-spacing: -.1px; color: #363c45; }
  .logo-mark {
    width: 25px; height: 25px; display: grid; place-items: center; border-radius: 8px;
    background: #e9effc; color: #526fae; font-size: 8.5px; font-weight: 700; letter-spacing: -.2px;
  }
  .panel-head-actions { display: flex; align-items: center; gap: 6px; }
  .count-badge {
    min-width: 21px; height: 20px; padding: 0 6px; display: grid; place-items: center;
    border-radius: 7px; color: var(--muted); background: #f4f5f8; font-size: 10.5px;
  }
  .close-btn {
    width: 26px; height: 26px; border: 0; border-radius: 8px; background: transparent; color: var(--faint);
    display: grid; place-items: center; transition: background-color var(--dur-1) var(--ease), color var(--dur-1) var(--ease);
  }
  .close-btn:hover { background: #f3f4f7; color: #414852; }
  .panel-subhead { display: flex; justify-content: space-between; align-items: center; padding: 0 16px 8px; color: var(--faint); font-size: 10.5px; }
  .live-hint { display: flex; align-items: center; gap: 5px; color: #6f9a84; }
  .pulse { width: 5px; height: 5px; border-radius: 50%; background: #76ae93; animation: pulse 2.4s var(--ease) infinite; }
  @keyframes pulse { 0%, 100% { box-shadow: 0 0 0 0 rgba(118, 174, 147, .3); } 50% { box-shadow: 0 0 0 3px rgba(118, 174, 147, 0); } }
  .panel-list { flex: 1; overflow-y: auto; padding: 2px 8px 8px; }
  .panel-row {
    display: flex; align-items: center; gap: 9px; padding: 9px 8px; border-radius: 10px; min-height: 50px;
    transition: background-color var(--dur-1) var(--ease);
  }
  .panel-row:hover { background: #f7f8fb; }
  .panel-row.disabled { opacity: .55; }
  .panel-row .status-dot { width: 6px; height: 6px; }
  .row-info { min-width: 0; flex: 1; }
  .row-name { font-size: 12.5px; font-weight: 550; color: #363c45; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row-meta { display: flex; gap: 8px; color: var(--faint); font-size: 10px; margin-top: 3px; text-transform: capitalize; }
  .row-error { color: #b76666; }
  .toggle-btn {
    width: 30px; height: 30px; flex: 0 0 auto; display: grid; place-items: center;
    border-radius: 50%; border: 0; background: #eef4fc; color: #5476b6;
    transition: background-color var(--dur-1) var(--ease), color var(--dur-1) var(--ease), transform var(--dur-1) var(--ease);
  }
  .toggle-btn:hover { background: #e3edfb; color: #42639f; }
  .toggle-btn:active { transform: scale(.94); }
  .toggle-btn.running { background: #fbf0f0; color: #b46060; }
  .toggle-btn.running:hover { background: #f9e5e5; }
  .btn-spinner { width: 11px; height: 11px; border: 1.5px solid #c6d4eb; border-top-color: #6484e8; border-radius: 50%; animation: spin .7s linear infinite; }
  .panel-loading, .panel-empty {
    height: 100%; min-height: 150px; display: flex; flex-direction: column; align-items: center;
    justify-content: center; gap: 8px; color: var(--faint); font-size: 11px;
  }
  .empty-mark { color: #bcc4cf; display: grid; place-items: center; }
  .panel-link { border: 0; background: transparent; color: #5f7fbd; font-size: 11px; padding: 4px; transition: color var(--dur-1) var(--ease); }
  .panel-link:hover { text-decoration: underline; color: #4a6aa6; }
  .panel-error {
    margin: 0 12px 7px; padding: 8px 10px; border: 1px solid #f2dddd; border-radius: 9px;
    color: #ad5c5c; background: #fffafa; font-size: 10.5px; line-height: 1.5;
  }
  .panel-foot { display: flex; gap: 7px; padding: 10px 12px 12px; border-top: 1px solid var(--line-soft); }
  .foot-btn {
    flex: 1; height: 32px; border-radius: 9px; border: 0; background: #f4f5f8; color: var(--muted); font-size: 11.5px;
    transition: background-color var(--dur-1) var(--ease), color var(--dur-1) var(--ease), transform var(--dur-1) var(--ease);
  }
  .foot-btn:hover { background: #eef0f4; color: #414852; }
  .foot-btn:active { transform: scale(.985); }
  .foot-btn.primary { color: #45649f; background: #e9effc; font-weight: 550; }
  .foot-btn.primary:hover { background: #dfe9fb; }
</style>
