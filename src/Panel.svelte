<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, friendlyError } from "./lib/api";
  import type { RunStateEvent, Script } from "./lib/types";

  let scripts: Script[] = $state([]);
  let loading = $state(true);
  let error = $state("");
  let busy = $state<Record<string, boolean>>({});

  const runningCount = $derived(scripts.filter((s) => s.status === "running" || s.status === "starting" || s.status === "queued").length);

  function isBusy(script: Script): boolean {
    return busy[script.id] === true || script.status === "starting" || script.status === "stopping" || script.status === "queued";
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
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    (async () => {
      const offState = await listen<RunStateEvent>("run-state", (event) => {
        const payload = event.payload;
        scripts = scripts.map((script) => script.id === payload.script_id
          ? { ...script, status: payload.status, last_error: payload.error, last_run_id: payload.run_id }
          : script);
      });
      const offChanged = await listen("scripts-changed", refresh);
      if (disposed) { offState(); offChanged(); } else { unlisteners.push(offState, offChanged); }
    })();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") closePanel();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      disposed = true;
      unlisteners.forEach((fn) => fn());
      window.removeEventListener("keydown", onKey);
    };
  });
</script>

<div class="panel-shell">
  <div class="panel-card">
    <header class="panel-head">
      <div class="panel-title">
        <span class="logo-dot"></span>
        <span>LiteScriptManager</span>
      </div>
      <div class="panel-head-actions">
        <span class="count-badge">{scripts.length}</span>
        <button class="close-btn" title="关闭 (Esc)" onclick={closePanel}>×</button>
      </div>
    </header>

    <div class="panel-subhead">
      <span>{runningCount > 0 ? `${runningCount} 个运行中` : "已配置脚本"}</span>
      <span class="live-hint"><span class="pulse"></span>实时状态</span>
    </div>

    <main class="panel-list">
      {#if loading}
        <div class="panel-loading"><span class="spinner"></span>加载中…</div>
      {:else if scripts.length === 0}
        <div class="panel-empty">
          <div class="empty-mark">▤</div>
          <div>还没有脚本</div>
          <button class="panel-link" onclick={openMain}>打开管理添加 →</button>
        </div>
      {:else}
        {#each scripts as script (script.id)}
          <div class="panel-row" class:disabled={!script.enabled}>
            <span class="status-dot {script.enabled ? script.status : 'disabled'}"></span>
            <div class="row-info">
              <div class="row-name" title={script.name}>{script.name}</div>
              <div class="row-meta">
                <span>{script.runtime_id}</span>
                {#if script.last_error}<span class="row-error" title={script.last_error}>上次失败</span>{/if}
              </div>
            </div>
            <button
              class="toggle-btn"
              class:running={script.status === "running"}
              disabled={!script.enabled || isBusy(script)}
              title={script.status === "running" ? "停止脚本" : "启动脚本"}
              onclick={() => toggle(script)}
            >
              {#if isBusy(script)}<span class="btn-spinner"></span>
              {:else if script.status === "running"}<span class="stop-icon"></span>
              {:else}<span class="play-icon"></span>{/if}
            </button>
          </div>
        {/each}
      {/if}
    </main>

    {#if error}
      <div class="panel-error">{error}</div>
    {/if}

    <footer class="panel-foot">
      <button class="foot-btn" onclick={stopAll}>全部停止</button>
      <button class="foot-btn primary" onclick={openMain}>打开管理 →</button>
    </footer>
  </div>
</div>

<style>
  :global(html), :global(body) { height: 100%; }
  .panel-shell { height: 100vh; padding: 7px; display: flex; }
  .panel-card {
    width: 100%; height: 100%; display: flex; flex-direction: column; overflow: hidden;
    background: rgba(16, 21, 29, .96); border: 1px solid #2b3545; border-radius: 14px;
    box-shadow: 0 18px 55px rgba(0, 0, 0, .5); backdrop-filter: blur(18px);
  }
  .panel-head { display: flex; align-items: center; justify-content: space-between; padding: 15px 15px 10px; }
  .panel-title { display: flex; align-items: center; gap: 8px; font-size: 12.5px; font-weight: 700; letter-spacing: .1px; }
  .logo-dot { width: 9px; height: 9px; border-radius: 3px; background: linear-gradient(145deg, #5eead4, #2dd4bf); box-shadow: 0 0 10px rgba(94,234,212,.4); }
  .panel-head-actions { display: flex; align-items: center; gap: 7px; }
  .count-badge { min-width: 20px; height: 18px; padding: 0 5px; display: grid; place-items: center; border: 1px solid #2a3545; border-radius: 5px; color: #7f8da4; font-size: 10px; }
  .close-btn { width: 23px; height: 23px; border: 0; border-radius: 6px; background: transparent; color: #7e8a9e; font-size: 18px; line-height: 1; }
  .close-btn:hover { background: #202936; color: #fff; }
  .panel-subhead { display: flex; justify-content: space-between; align-items: center; padding: 0 15px 8px; color: #77849a; font-size: 10px; }
  .live-hint { display: flex; align-items: center; gap: 5px; color: #58736e; }
  .pulse { width: 5px; height: 5px; border-radius: 50%; background: #34d399; box-shadow: 0 0 0 3px rgba(52,211,153,.1); }
  .panel-list { flex: 1; overflow-y: auto; padding: 2px 8px 8px; }
  .panel-row { display: flex; align-items: center; gap: 9px; padding: 9px 8px; border-radius: 8px; min-height: 48px; }
  .panel-row:hover { background: #18212d; }
  .panel-row.disabled { opacity: .55; }
  .panel-row .status-dot { width: 7px; height: 7px; }
  .row-info { min-width: 0; flex: 1; }
  .row-name { font-size: 12px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row-meta { display: flex; gap: 7px; color: #627086; font-size: 9.5px; margin-top: 4px; text-transform: capitalize; }
  .row-error { color: #fca5a5; }
  .toggle-btn { width: 28px; height: 28px; flex: 0 0 auto; display: grid; place-items: center; border-radius: 50%; border: 1px solid #314053; background: #1b2634; color: #9fb0c5; }
  .toggle-btn:hover { border-color: #4a786f; color: #d4fff7; background: #1a2b2a; }
  .toggle-btn.running { border-color: #4a2529; background: #2a171a; color: #fca5a5; }
  .toggle-btn.running:hover { background: #3a1e23; }
  .play-icon { width: 0; height: 0; border-top: 5px solid transparent; border-bottom: 5px solid transparent; border-left: 7px solid currentColor; margin-left: 2px; }
  .stop-icon { width: 8px; height: 8px; border-radius: 1.5px; background: currentColor; }
  .btn-spinner { width: 11px; height: 11px; border: 1.5px solid #526276; border-top-color: #5eead4; border-radius: 50%; animation: spin .7s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .panel-loading, .panel-empty { height: 100%; min-height: 150px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; color: #758298; font-size: 11px; }
  .empty-mark { font-size: 26px; color: #3e4c60; }
  .panel-link { border: 0; background: transparent; color: #5eead4; font-size: 11px; padding: 4px; }
  .panel-link:hover { text-decoration: underline; }
  .panel-error { margin: 0 12px 7px; padding: 7px 9px; border: 1px solid #5b2a30; border-radius: 6px; color: #fca5a5; font-size: 10px; line-height: 1.4; }
  .panel-foot { display: flex; gap: 7px; padding: 10px 12px 12px; border-top: 1px solid #222b39; }
  .foot-btn { flex: 1; height: 30px; border-radius: 7px; border: 1px solid #2b3647; background: #151d29; color: #8592a6; font-size: 11px; }
  .foot-btn:hover { background: #1c2634; color: #dbe4f0; }
  .foot-btn.primary { color: #06241f; background: #5eead4; border-color: #5eead4; font-weight: 650; }
  .foot-btn.primary:hover { background: #7df3df; }
</style>
