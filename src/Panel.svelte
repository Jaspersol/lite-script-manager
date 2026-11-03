<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, friendlyError } from "./lib/api";
  import Icon from "./lib/Icon.svelte";
  import type { RunStateEvent, Script } from "./lib/types";

  let scripts: Script[] = $state([]);
  let loading = $state(true);
  let error = $state("");
  let busy = $state<Record<string, boolean>>({});

  const runningCount = $derived(scripts.filter((s) => s.status === "running" || s.status === "starting" || s.status === "queued").length);

  const runtimeLabels: Record<string, string> = {
    python: "Python",
    javascript: "JavaScript",
    shell: "Shell",
    perl: "Perl",
    php: "PHP",
    r: "R",
    lua: "Lua",
    batch: "Batch",
    powershell: "PowerShell",
    vbscript: "VBScript",
    jscript: "JScript",
    wsf: "WSF",
    autohotkey: "AutoHotkey",
    autoit: "AutoIt",
  };

  function runtimeLabel(id: string): string {
    return runtimeLabels[id] ?? id;
  }

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
        <span class="logo-mark">LS</span>
        <span>LiteScriptManager</span>
      </div>
      <div class="panel-head-actions">
        <span class="count-badge">{scripts.length}</span>
        <button class="close-btn" title="关闭 (Esc)" aria-label="关闭" onclick={closePanel}><Icon name="close" size={16} /></button>
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
          <div class="empty-mark"><Icon name="grid" size={28} strokeWidth={1.3} /></div>
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
                <span>{runtimeLabel(script.runtime_id)}</span>
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
      <button class="foot-btn" onclick={stopAll}>全部停止</button>
      <button class="foot-btn primary" onclick={openMain}>打开管理</button>
    </footer>
  </div>
</div>

<style>
  :global(html), :global(body) { height: 100%; background: transparent; }
  .panel-shell { height: 100vh; padding: 8px; display: flex; }
  .panel-card {
    width: 100%; height: 100%; display: flex; flex-direction: column; overflow: hidden;
    background: rgba(255, 255, 255, .97); border: 1px solid #e7e9ef; border-radius: 18px;
    box-shadow: 0 14px 44px rgba(32, 41, 58, .14); backdrop-filter: blur(18px);
  }
  .panel-head { display: flex; align-items: center; justify-content: space-between; padding: 16px 16px 10px; }
  .panel-title { display: flex; align-items: center; gap: 9px; font-size: 12.5px; font-weight: 600; letter-spacing: -.1px; color: #3b414a; }
  .logo-mark {
    width: 24px; height: 24px; display: grid; place-items: center; border-radius: 7px;
    background: #e9effc; color: #526fae; font-size: 8px; font-weight: 700; letter-spacing: -.2px;
  }
  .panel-head-actions { display: flex; align-items: center; gap: 7px; }
  .count-badge { min-width: 20px; height: 19px; padding: 0 6px; display: grid; place-items: center; border-radius: 7px; color: #8b929e; background: #f4f5f8; font-size: 9.5px; }
  .close-btn { width: 26px; height: 26px; border: 0; border-radius: 8px; background: transparent; color: #9aa1ac; display: grid; place-items: center; }
  .close-btn:hover { background: #f3f4f7; color: #4e5560; }
  .panel-subhead { display: flex; justify-content: space-between; align-items: center; padding: 0 16px 9px; color: #a3a9b3; font-size: 10px; }
  .live-hint { display: flex; align-items: center; gap: 5px; color: #8ba99a; }
  .pulse { width: 5px; height: 5px; border-radius: 50%; background: #76ae93; }
  .panel-list { flex: 1; overflow-y: auto; padding: 2px 8px 8px; }
  .panel-row { display: flex; align-items: center; gap: 9px; padding: 10px 8px; border-radius: 11px; min-height: 50px; transition: background .12s; }
  .panel-row:hover { background: #f8f9fb; }
  .panel-row.disabled { opacity: .52; }
  .panel-row .status-dot { width: 6px; height: 6px; }
  .row-info { min-width: 0; flex: 1; }
  .row-name { font-size: 12px; font-weight: 550; color: #3d434c; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row-meta { display: flex; gap: 7px; color: #a0a6b0; font-size: 9.5px; margin-top: 4px; text-transform: capitalize; }
  .row-error { color: #b76666; }
  .toggle-btn {
    width: 29px; height: 29px; flex: 0 0 auto; display: grid; place-items: center;
    border-radius: 50%; border: 0; background: #eef4fc; color: #5476b6; transition: background .15s, color .15s;
  }
  .toggle-btn:hover { background: #e3edfb; color: #42639f; }
  .toggle-btn.running { background: #fbf0f0; color: #b46060; }
  .toggle-btn.running:hover { background: #f9e5e5; }
  .btn-spinner { width: 11px; height: 11px; border: 1.5px solid #c6d4eb; border-top-color: #6484e8; border-radius: 50%; animation: spin .7s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .panel-loading, .panel-empty { height: 100%; min-height: 150px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; color: #9ba2ad; font-size: 10.5px; }
  .empty-mark { color: #bcc4cf; display: grid; place-items: center; }
  .panel-link { border: 0; background: transparent; color: #5f7fbd; font-size: 10.5px; padding: 4px; }
  .panel-link:hover { text-decoration: underline; }
  .panel-error { margin: 0 12px 7px; padding: 8px 10px; border: 1px solid #f2dddd; border-radius: 9px; color: #ad5c5c; background: #fffafa; font-size: 9.5px; line-height: 1.45; }
  .panel-foot { display: flex; gap: 7px; padding: 11px 12px 12px; border-top: 1px solid #f1f2f5; }
  .foot-btn { flex: 1; height: 31px; border-radius: 9px; border: 0; background: #f5f6f8; color: #7c8490; font-size: 11px; }
  .foot-btn:hover { background: #eef0f4; color: #555d69; }
  .foot-btn.primary { color: #45649f; background: #e9effc; font-weight: 550; }
  .foot-btn.primary:hover { background: #dfe9fb; }
</style>
