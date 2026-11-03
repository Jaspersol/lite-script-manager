<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, formatBytes, formatDuration, formatTime, friendlyError } from "./api";
  import type { LogBatchEvent, RunRecord, RunStateEvent, Script } from "./types";

  let {
    scripts = [],
    notify = () => {},
  }: {
    scripts: Script[];
    notify: (message: string, kind?: "info" | "error") => void;
  } = $props();

  let runs: RunRecord[] = $state([]);
  let loading = $state(true);
  let scriptFilter = $state("");
  let statusFilter = $state("");
  let sourceFilter = $state("");
  let selected: RunRecord | null = $state(null);
  let logLines: string[] = $state([]);
  let logLoading = $state(false);

  function sourceLabel(value: string): string {
    return {
      manual: "手动",
      schedule: "定时",
      loop: "循环",
      trigger: "触发器",
      adopted: "接管",
      queue: "排队",
    }[value] ?? value;
  }

  function statusLabel(value: string): string {
    return {
      running: "运行中",
      queued: "排队中",
      starting: "启动中",
      stopping: "停止中",
      stopped: "已停止",
      exited: "已结束",
      failed: "失败",
    }[value] ?? value;
  }

  async function load() {
    try {
      runs = await api.listRuns({
        scriptId: scriptFilter || null,
        status: statusFilter || null,
        source: sourceFilter || null,
        limit: 1000,
      });
      if (selected) {
        selected = runs.find((run) => run.id === selected?.id) ?? selected;
      }
    } catch (error) {
      notify(friendlyError(error), "error");
    } finally {
      loading = false;
    }
  }

  async function selectRun(run: RunRecord) {
    selected = run;
    logLines = [];
    logLoading = true;
    try {
      await api.setLogStreaming(false);
      logLines = await api.getRunLog(run.id, 500);
      if (run.status === "running" || run.status === "starting" || run.status === "stopping") {
        await api.setLogStreaming(true);
      }
    } catch (error) {
      notify(friendlyError(error), "error");
    } finally {
      logLoading = false;
    }
  }

  async function openFullLog(run: RunRecord) {
    try {
      await api.openLogFile(run.id);
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  async function clearHistory() {
    if (!confirm("清空全部运行历史？日志文件仍会保留，可在设置中调整日志保留策略。")) return;
    try {
      const count = await api.clearRuns(null);
      selected = null;
      logLines = [];
      await api.setLogStreaming(false);
      await load();
      notify(`已清理 ${count} 条历史记录`);
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  function setFilter() {
    selected = null;
    logLines = [];
    load();
  }

  onMount(() => {
    load();
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    (async () => {
      let timer: ReturnType<typeof setTimeout> | null = null;
      const refresh = () => {
        if (timer) clearTimeout(timer);
        timer = setTimeout(load, 300);
      };
      const offRuns = await listen("runs-changed", refresh);
      const offState = await listen<RunStateEvent>("run-state", (event) => {
        if (selected?.id === event.payload.run_id && !["running", "starting", "stopping"].includes(event.payload.status)) {
          api.setLogStreaming(false).catch(() => {});
        }
        refresh();
      });
      const offLog = await listen<LogBatchEvent>("run-log-batch", (event) => {
        if (selected?.id !== event.payload.run_id) return;
        const lines = event.payload.lines.map((line) => `[${line.stream}] ${line.line}`);
        logLines = [...logLines, ...lines].slice(-1000);
      });
      if (disposed) {
        offRuns();
        offState();
        offLog();
      } else {
        unlisteners.push(offRuns, offState, offLog);
      }
    })();
    return () => {
      disposed = true;
      api.setLogStreaming(false).catch(() => {});
      unlisteners.forEach((fn) => fn());
    };
  });
</script>

<section class="content">
  <div class="section-actions">
    <button class="btn btn-danger" onclick={clearHistory}>清空历史</button>
  </div>

  <div class="filter-row">
    <select class="select" bind:value={scriptFilter} onchange={setFilter}>
      <option value="">全部脚本</option>
      {#each scripts as script (script.id)}<option value={script.id}>{script.name}</option>{/each}
    </select>
    <select class="select" bind:value={statusFilter} onchange={setFilter}>
      <option value="">全部状态</option>
      <option value="running">运行中</option><option value="queued">排队中</option><option value="stopped">已停止</option><option value="exited">已结束</option><option value="failed">失败</option>
    </select>
    <select class="select" bind:value={sourceFilter} onchange={setFilter}>
      <option value="">全部来源</option>
      <option value="manual">手动</option><option value="schedule">定时</option><option value="loop">循环</option><option value="trigger">触发器</option><option value="adopted">接管</option>
    </select>
    <button class="btn" onclick={load}>刷新</button>
  </div>

  <div class="history-layout">
    <div class="history-list">
      {#if loading}
        <div class="empty-state small"><div class="spinner"></div>正在加载历史…</div>
      {:else if runs.length === 0}
        <div class="empty-state small"><h2>没有运行记录</h2><p>手动或自动运行脚本后会出现在这里。</p></div>
      {:else}
        <div class="table-card">
          <div class="table-head run-grid"><span>开始时间</span><span>脚本</span><span>来源</span><span>状态</span><span>耗时</span><span>退出码</span><span class="right">日志</span></div>
          {#each runs as run (run.id)}
            <button class="table-row run-grid run-row" class:selected={selected?.id === run.id} onclick={() => selectRun(run)}>
              <span class="cell-sub">{formatTime(run.started_at)}</span>
              <span class="ellipsis">{run.script_name}</span>
              <span class="badge">{sourceLabel(run.source)}</span>
              <span><span class="badge" class:ok={run.status === "running" || run.status === "exited"} class:bad={run.status === "failed"}>{statusLabel(run.status)}</span></span>
              <span class="cell-sub">{formatDuration(run.started_at, run.ended_at)}</span>
              <span class="mono tiny">{run.exit_code ?? "—"}</span>
              <span class="right"><span class="btn btn-sm" role="button" tabindex="0" onclick={(event) => { event.stopPropagation(); openFullLog(run); }} onkeydown={(event) => { if (event.key === "Enter") { event.stopPropagation(); openFullLog(run); } }}>打开</span></span>
            </button>
          {/each}
        </div>
      {/if}
    </div>

    <aside class="detail-panel">
      {#if selected}
        <div class="detail-head">
          <div>
            <div class="detail-title">{selected.script_name}</div>
            <div class="cell-sub mono">{selected.id}</div>
          </div>
          <span class="badge">{statusLabel(selected.status)}</span>
        </div>
        <div class="detail-grid">
          <span>来源</span><strong>{sourceLabel(selected.source)}</strong>
          <span>PID</span><strong>{selected.pid ?? "—"}</strong>
          <span>耗时</span><strong>{formatDuration(selected.started_at, selected.ended_at)}</strong>
          <span>退出码</span><strong>{selected.exit_code ?? "—"}</strong>
          <span>停止原因</span><strong>{selected.stop_reason ?? "—"}</strong>
          <span>峰值内存</span><strong>{formatBytes(selected.peak_memory_bytes)}</strong>
          <span>CPU 时间</span><strong>{selected.cpu_time_ms != null ? `${(selected.cpu_time_ms / 1000).toFixed(1)}s` : "—"}</strong>
          <span>日志大小</span><strong>{formatBytes(selected.log_bytes)}</strong>
        </div>
        {#if selected.error}<div class="detail-error">{selected.error}</div>{/if}
        <div class="detail-head log-head"><strong>日志尾部</strong><button class="btn btn-sm" onclick={() => openFullLog(selected!)}>打开完整日志</button></div>
        <div class="log-view mono">
          {#if logLoading}
            <div class="log-empty">正在读取…</div>
          {:else if logLines.length === 0}
            <div class="log-empty">没有日志输出。</div>
          {:else}
            {#each logLines as line, index (index)}<div class="log-line">{line}</div>{/each}
          {/if}
        </div>
      {:else}
        <div class="detail-placeholder">
          <div class="empty-icon">▤</div>
          <p>选择一条运行记录查看退出码、耗时和日志尾部。</p>
        </div>
      {/if}
    </aside>
  </div>
</section>

<style>
  .section-actions { display: flex; justify-content: flex-end; margin-bottom: 12px; }
  .filter-row { display: flex; gap: 7px; margin-bottom: 12px; }
  .filter-row .select { width: 150px; height: 32px; padding: 4px 8px; font-size: 12px; }
  .history-layout { display: grid; grid-template-columns: minmax(0, 1.55fr) minmax(280px, .8fr); gap: 12px; align-items: start; }
  .history-list { min-width: 0; }
  .table-card { border: 1px solid var(--line-soft); border-radius: 11px; overflow: hidden; background: #111720; }
  .run-grid { display: grid; grid-template-columns: 132px minmax(120px, 1fr) 62px 70px 62px 52px 48px; gap: 8px; align-items: center; }
  .table-head { padding: 9px 12px; color: var(--muted-2); font-size: 9.5px; letter-spacing: .07em; text-transform: uppercase; background: #0e141d; border-bottom: 1px solid var(--line-soft); }
  .table-row { width: 100%; text-align: left; padding: 9px 12px; border: 0; border-bottom: 1px solid #1b222c; background: transparent; color: var(--text); font-size: 11px; }
  .table-row:last-child { border-bottom: 0; }
  .table-row:hover { background: #151d28; }
  .run-row { cursor: pointer; }
  .run-row.selected { background: #17242a; box-shadow: inset 2px 0 0 var(--accent); }
  .cell-sub { color: var(--muted); font-size: 10px; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .badge.ok { color: #a7f3d0; border-color: #174b3e; }
  .badge.bad { color: #fecaca; border-color: #4a2529; }
  .right { text-align: right; }
  .detail-panel { border: 1px solid var(--line-soft); border-radius: 11px; background: #111720; padding: 14px; min-height: 320px; }
  .detail-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 10px; }
  .detail-title { font-size: 14px; font-weight: 650; }
  .detail-grid { display: grid; grid-template-columns: 78px 1fr; gap: 7px 10px; margin: 14px 0; font-size: 11px; }
  .detail-grid span { color: var(--muted-2); }
  .detail-grid strong { font-weight: 500; color: var(--text); }
  .detail-error { padding: 8px 10px; border: 1px solid #4a2529; border-radius: 7px; color: #fecaca; background: #22171a; font-size: 10.5px; line-height: 1.45; margin-bottom: 12px; }
  .log-head { margin-top: 8px; margin-bottom: 8px; align-items: center; }
  .log-view { height: 240px; overflow: auto; background: #0b1016; border: 1px solid #1c2531; border-radius: 8px; padding: 8px; font-size: 10px; line-height: 1.55; }
  .log-line { white-space: pre-wrap; word-break: break-word; color: #9fb0c7; }
  .log-line:nth-child(odd) { color: #8fa1b9; }
  .log-empty { color: var(--muted); padding: 8px; }
  .detail-placeholder { min-height: 280px; display: flex; align-items: center; justify-content: center; flex-direction: column; color: var(--muted); font-size: 11px; text-align: center; }
  .empty-icon { color: #3c4b60; font-size: 28px; }
  .empty-state.small { min-height: 240px; }
  @media (max-width: 1100px) {
    .history-layout { grid-template-columns: 1fr; }
  }
  @media (max-width: 760px) {
    .run-grid { grid-template-columns: 120px minmax(100px, 1fr) 55px 65px 55px; }
    .run-grid > :nth-child(6), .run-grid > :nth-child(7) { display: none; }
  }
</style>
