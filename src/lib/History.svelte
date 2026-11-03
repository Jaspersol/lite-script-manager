<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, formatBytes, formatDuration, formatTime, friendlyError } from "./api";
  import Icon from "./Icon.svelte";
  import { pageAction } from "./uiActions";
  import type { PageAction } from "./uiActions";
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

  $effect(() => {
    const action: PageAction = {
      label: "清空历史",
      icon: "trash",
      kind: "danger",
      run: clearHistory,
    };
    pageAction.set(action);
    return () => pageAction.set(null);
  });

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

<div>
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
    <button class="btn" onclick={load}><Icon name="refresh" size={14} /> 刷新</button>
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
              <span class="right"><span class="log-open" role="button" tabindex="0" title="打开完整日志" aria-label="打开完整日志" onclick={(event) => { event.stopPropagation(); openFullLog(run); }} onkeydown={(event) => { if (event.key === "Enter") { event.stopPropagation(); openFullLog(run); } }}><Icon name="file" size={14} /></span></span>
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
          <div class="empty-icon"><Icon name="file" size={28} strokeWidth={1.25} /></div>
          <p>选择一条运行记录查看退出码、耗时和日志尾部。</p>
        </div>
      {/if}
    </aside>
  </div>
</div>

<style>
  .filter-row { display: flex; gap: 7px; margin-bottom: 13px; flex-wrap: wrap; }
  .filter-row .select { width: 138px; height: 34px; padding: 5px 9px; font-size: 11.5px; }
  .history-layout { display: grid; grid-template-columns: minmax(0, 1.48fr) minmax(264px, .78fr); gap: 12px; align-items: start; }
  .history-list { min-width: 0; }
  .table-card { border: 1px solid var(--line-soft); border-radius: 16px; overflow: hidden; background: #fff; }
  .run-grid { display: grid; grid-template-columns: 110px minmax(72px, 1fr) 46px 52px 44px 38px 30px; gap: 6px; align-items: center; }
  .table-head {
    padding: 11px 13px; color: #8b929d; font-size: 11px; font-weight: 500;
    letter-spacing: .01em; background: #fafbfc; border-bottom: 1px solid var(--line-soft);
  }
  .table-row {
    width: 100%; text-align: left; padding: 10px 13px; border: 0;
    border-bottom: 1px solid #f3f4f7; background: transparent; color: #4a515c; font-size: 11.5px;
    transition: background-color var(--dur-1) var(--ease);
  }
  .table-row:last-child { border-bottom: 0; }
  .table-row:hover { background: #fafbfd; }
  .run-row { cursor: pointer; }
  .run-row.selected { background: #eef4fc; }
  .cell-sub { color: #858c98; font-size: 10.5px; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .right { text-align: right; }
  .log-open {
    width: 26px; height: 26px; display: inline-grid; place-items: center;
    border-radius: 8px; color: #8f96a1; cursor: pointer;
    transition: background-color var(--dur-1) var(--ease), color var(--dur-1) var(--ease);
  }
  .log-open:hover { background: #f1f3f7; color: #5b7bb9; }
  .detail-panel { border: 1px solid var(--line-soft); border-radius: 16px; background: #fff; padding: 17px; min-height: 320px; }
  @media (min-width: 1101px) {
    .detail-panel { position: sticky; top: 0; }
  }
  .detail-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 10px; }
  .detail-title { font-size: 13.5px; font-weight: 550; color: #363c45; }
  .detail-grid { display: grid; grid-template-columns: 70px 1fr; gap: 9px 10px; margin: 16px 0; font-size: 11.5px; }
  .detail-grid span { color: #8f96a1; }
  .detail-grid strong { font-weight: 500; color: #4b525d; }
  .detail-error { padding: 9px 11px; border: 1px solid #f2dddd; border-radius: 10px; color: #ad5c5c; background: #fffafa; font-size: 11px; line-height: 1.5; margin-bottom: 13px; }
  .log-head { margin-top: 10px; margin-bottom: 9px; align-items: center; }
  .log-view {
    height: 250px; overflow: auto; background: #f8f9fb; border: 1px solid var(--line-soft);
    border-radius: 11px; padding: 11px 12px; font-size: 11px; line-height: 1.65;
  }
  .log-line { white-space: pre-wrap; word-break: break-word; color: #5b6675; }
  .log-empty { color: #8f96a1; padding: 8px; }
  .detail-placeholder { min-height: 280px; display: flex; align-items: center; justify-content: center; flex-direction: column; color: #8f96a1; font-size: 11.5px; text-align: center; }
  .detail-placeholder p { max-width: 210px; line-height: 1.6; }
  .empty-icon { color: #bdc5d0; display: grid; place-items: center; }
  .empty-state.small { min-height: 240px; }
  @media (max-width: 1100px) {
    .history-layout { grid-template-columns: 1fr; }
  }
  @media (max-width: 760px) {
    .run-grid { grid-template-columns: 92px minmax(80px, 1fr) 46px 52px 44px; }
    .run-grid > :nth-child(6), .run-grid > :nth-child(7) { display: none; }
  }
</style>
