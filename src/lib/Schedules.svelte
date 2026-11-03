<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, formatTime, friendlyError } from "./api";
  import type { Schedule, ScheduleEdit, Script } from "./types";

  let {
    scripts = [],
    notify = () => {},
  }: {
    scripts: Script[];
    notify: (message: string, kind?: "info" | "error") => void;
  } = $props();

  let schedules: Schedule[] = $state([]);
  let loading = $state(true);
  let editing: Schedule | null = $state(null);
  let edit: ScheduleEdit | null = $state(null);
  let preview: number[] = $state([]);
  let previewError = $state("");

  const kindLabels: Record<string, string> = {
    cron: "Cron",
    interval: "固定间隔",
    once: "一次性",
    loop: "循环重启",
  };

  function scriptName(id: string): string {
    return scripts.find((item) => item.id === id)?.name ?? "已删除脚本";
  }

  function editFrom(schedule: Schedule): ScheduleEdit {
    return {
      name: schedule.name,
      script_id: schedule.script_id,
      kind: schedule.kind,
      expression: schedule.expression,
      timezone: schedule.timezone,
      enabled: schedule.enabled,
      overlap_policy: schedule.overlap_policy,
      catch_up: schedule.catch_up,
      timeout_sec: schedule.timeout_sec,
      success_codes: schedule.success_codes,
      restart_delay_sec: schedule.restart_delay_sec,
      max_restarts: schedule.max_restarts,
      backoff: schedule.backoff,
    };
  }

  function openNew() {
    editing = null;
    edit = {
      name: "",
      script_id: scripts[0]?.id ?? "",
      kind: "cron",
      expression: "0 9 * * *",
      timezone: "local",
      enabled: true,
      overlap_policy: "skip",
      catch_up: "none",
      timeout_sec: null,
      success_codes: "0",
      restart_delay_sec: 0,
      max_restarts: 0,
      backoff: true,
    };
    preview = [];
    previewError = "";
  }

  function openEdit(schedule: Schedule) {
    editing = schedule;
    edit = editFrom(schedule);
    preview = [];
    previewError = "";
  }

  async function load() {
    try {
      schedules = await api.listSchedules();
    } catch (error) {
      notify(friendlyError(error), "error");
    } finally {
      loading = false;
    }
  }

  async function save() {
    if (!edit) return;
    if (!edit.name.trim()) {
      notify("请填写计划名称", "error");
      return;
    }
    if (!edit.script_id) {
      notify("请选择脚本", "error");
      return;
    }
    try {
      if (editing) await api.updateSchedule(editing.id, edit);
      else await api.createSchedule(edit);
      edit = null;
      editing = null;
      await load();
      notify("计划已保存");
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  async function remove(schedule: Schedule) {
    if (!confirm(`删除计划「${schedule.name}」？`)) return;
    try {
      await api.deleteSchedule(schedule.id);
      await load();
      notify("计划已删除");
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  async function toggle(schedule: Schedule) {
    const next = editFrom(schedule);
    next.enabled = !next.enabled;
    try {
      await api.updateSchedule(schedule.id, next);
      await load();
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  async function runNow(schedule: Schedule) {
    try {
      await api.runScheduleNow(schedule.id);
      notify(`已触发「${schedule.name}」`);
      await load();
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  async function updatePreview() {
    if (!edit) return;
    previewError = "";
    if (edit.kind === "loop") {
      preview = [];
      return;
    }
    try {
      const result = await api.previewSchedule(edit.kind, edit.expression, edit.timezone, 5);
      preview = result.next;
      previewError = result.error ?? "";
    } catch (error) {
      previewError = friendlyError(error);
    }
  }

  function applyQuick(value: string) {
    if (!edit) return;
    edit.kind = "cron";
    edit.expression = value;
  }

  const conflicts = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const schedule of schedules) {
      if (schedule.enabled) counts.set(schedule.script_id, (counts.get(schedule.script_id) ?? 0) + 1);
    }
    return new Set([...counts.entries()].filter(([, count]) => count > 1).map(([id]) => id));
  });

  function statusLabel(status: string | null): string {
    return {
      running: "运行中",
      queued: "排队中",
      stopped: "已停止",
      exited: "已结束",
      failed: "失败",
      skipped: "已跳过",
    }[status ?? ""] ?? "未运行";
  }

  onMount(() => {
    load();
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    (async () => {
      const off = await listen("schedules-changed", load);
      if (disposed) off(); else unlisteners.push(off);
    })();
    return () => {
      disposed = true;
      unlisteners.forEach((fn) => fn());
    };
  });
</script>

<section class="content">
  <div class="section-actions">
    <button class="btn btn-primary" onclick={openNew} disabled={scripts.length === 0}>+ 新建计划</button>
  </div>

  {#if loading}
    <div class="empty-state"><div class="spinner"></div>正在加载计划…</div>
  {:else if schedules.length === 0}
    <div class="empty-state">
      <div class="empty-icon">◷</div>
      <h2>还没有计划任务</h2>
      <p>为脚本设置固定间隔、Cron 或后台循环重启。</p>
      <button class="btn btn-primary" onclick={openNew} disabled={scripts.length === 0}>+ 新建计划</button>
    </div>
  {:else}
    <div class="table-card">
      <div class="table-head schedule-grid">
        <span>计划</span><span>脚本</span><span>规则</span><span>下次运行</span><span>上次结果</span><span class="right">操作</span>
      </div>
      {#each schedules as schedule (schedule.id)}
        <div class="table-row schedule-grid" class:disabled-row={!schedule.enabled}>
          <div class="cell-main">
            <div class="cell-title">
              {schedule.name}
              {#if conflicts.has(schedule.script_id)}<span class="badge warn" title="同一脚本存在多个启用计划">冲突</span>{/if}
            </div>
            <div class="cell-sub">{kindLabels[schedule.kind] ?? schedule.kind} · {schedule.timezone}</div>
          </div>
          <div class="ellipsis">{scriptName(schedule.script_id)}</div>
          <div class="mono expression">{schedule.expression}</div>
          <div class="cell-sub">{schedule.enabled ? formatTime(schedule.next_run_at) : "已禁用"}</div>
          <div>
            <span class="badge" class:ok={schedule.last_status === "running" || schedule.last_status === "exited"} class:bad={schedule.last_status === "failed"}>{statusLabel(schedule.last_status)}</span>
            {#if schedule.last_error}<div class="cell-error" title={schedule.last_error}>{schedule.last_error}</div>{/if}
          </div>
          <div class="row-actions">
            <button class="btn btn-sm" onclick={() => runNow(schedule)}>立即执行</button>
            <button class="btn btn-sm" onclick={() => toggle(schedule)}>{schedule.enabled ? "禁用" : "启用"}</button>
            <button class="btn btn-sm btn-ghost" onclick={() => openEdit(schedule)}>编辑</button>
            <button class="icon-btn btn btn-ghost" title="删除" onclick={() => remove(schedule)}>×</button>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</section>

{#if edit}
  <div class="modal-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) edit = null; }}>
    <div class="modal wide" role="dialog" aria-modal="true" aria-label="编辑计划">
      <div class="modal-head">
        <div class="modal-title">{editing ? "编辑计划" : "新建计划"}</div>
        <button class="btn btn-ghost icon-btn" onclick={() => { edit = null; editing = null; }}>×</button>
      </div>
      <div class="modal-body">
        <div class="form-grid">
          <div class="field">
            <label class="label" for="schedule-name">名称</label>
            <input id="schedule-name" class="input" bind:value={edit.name} placeholder="例如：每日备份" />
          </div>
          <div class="field">
            <label class="label" for="schedule-script">脚本</label>
            <select id="schedule-script" class="select" bind:value={edit.script_id}>
              {#each scripts as script (script.id)}<option value={script.id}>{script.name}</option>{/each}
            </select>
          </div>
        </div>

        <div class="form-grid">
          <div class="field">
            <label class="label" for="schedule-kind">类型</label>
            <select id="schedule-kind" class="select" bind:value={edit.kind}>
              <option value="cron">Cron</option>
              <option value="interval">固定间隔</option>
              <option value="once">一次性</option>
              <option value="loop">退出后循环重启</option>
            </select>
          </div>
          <div class="field">
            <label class="label" for="schedule-timezone">时区</label>
            <input id="schedule-timezone" class="input mono" bind:value={edit.timezone} placeholder="local / UTC / Asia/Shanghai" />
          </div>
        </div>

        <div class="field">
          <label class="label" for="schedule-expression">
            {edit.kind === "cron" ? "Cron 表达式（5 段或 6 段）" : edit.kind === "interval" ? "间隔（如 30s、5m、1h）" : edit.kind === "once" ? "执行时间（2026-01-01 12:00 或 RFC3339）" : "循环间隔（如 10s、5m）"}
          </label>
          <input id="schedule-expression" class="input mono" bind:value={edit.expression} />
        </div>
        {#if edit.kind === "cron"}
          <div class="quick-row">
            <span class="tiny muted">快捷：</span>
            <button class="btn btn-sm" onclick={() => applyQuick("0 9 * * *")}>每天 09:00</button>
            <button class="btn btn-sm" onclick={() => applyQuick("0 9 * * 1")}>每周一 09:00</button>
            <button class="btn btn-sm" onclick={() => applyQuick("0 * * * *")}>每小时</button>
            <button class="btn btn-sm" onclick={() => applyQuick("*/5 * * * *")}>每 5 分钟</button>
          </div>
        {/if}

        <div class="preview-box">
          <div class="preview-head">
            <strong>下次执行预览</strong>
            <button class="btn btn-sm" onclick={updatePreview} disabled={edit.kind === "loop"}>计算</button>
          </div>
          {#if previewError}<div class="cell-error">{previewError}</div>{/if}
          {#if preview.length}
            <div class="preview-list">{preview.map((value) => formatTime(value)).join("  ·  ")}</div>
          {:else if !previewError}
            <div class="tiny muted">{edit.kind === "loop" ? "循环计划在每次退出后按间隔重新启动。" : "点击“计算”验证表达式。"}</div>
          {/if}
        </div>

        <div class="form-grid">
          <div class="field">
            <label class="label" for="schedule-overlap">重叠策略</label>
            <select id="schedule-overlap" class="select" bind:value={edit.overlap_policy}>
              <option value="skip">跳过</option>
              <option value="queue">排队（延迟重试）</option>
              <option value="replace">停止旧实例后启动</option>
              <option value="parallel">允许并行</option>
            </select>
          </div>
          <div class="field">
            <label class="label" for="schedule-catchup">错过后</label>
            <select id="schedule-catchup" class="select" bind:value={edit.catch_up}>
              <option value="none">不补</option>
              <option value="once">补一次</option>
              <option value="all">补全部（最多 10 次）</option>
            </select>
          </div>
        </div>

        <div class="form-grid three">
          <div class="field">
            <label class="label" for="schedule-timeout">最大运行秒数</label>
            <input id="schedule-timeout" class="input" type="number" min="0" placeholder="留空使用脚本设置" bind:value={edit.timeout_sec} />
          </div>
          <div class="field">
            <label class="label" for="schedule-success">成功退出码</label>
            <input id="schedule-success" class="input mono" bind:value={edit.success_codes} placeholder="0 或 0,2,10-12" />
          </div>
          <div class="field">
            <label class="label" for="schedule-restarts">最大重启次数</label>
            <input id="schedule-restarts" class="input" type="number" min="0" bind:value={edit.max_restarts} />
          </div>
        </div>

        {#if edit.kind === "loop"}
          <div class="form-grid">
            <div class="field">
              <label class="label" for="schedule-delay">退出后延迟秒数</label>
              <input id="schedule-delay" class="input" type="number" min="0" bind:value={edit.restart_delay_sec} />
            </div>
            <label class="check-row"><input type="checkbox" bind:checked={edit.backoff} /> 失败时使用 1/2/5/10/30 秒指数退避</label>
          </div>
        {/if}

        <label class="check-row"><input type="checkbox" bind:checked={edit.enabled} /> 启用此计划</label>
      </div>
      <div class="modal-foot">
        <button class="btn" onclick={() => { edit = null; editing = null; }}>取消</button>
        <button class="btn btn-primary" onclick={save}>保存</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .section-actions { display: flex; justify-content: flex-end; margin-bottom: 14px; }
  .table-card { border: 1px solid var(--line-soft); border-radius: 11px; overflow: hidden; background: #111720; }
  .schedule-grid { display: grid; grid-template-columns: minmax(150px, 1.15fr) 130px minmax(180px, 1.3fr) 150px 130px 248px; gap: 12px; align-items: center; }
  .table-head { padding: 10px 14px; color: var(--muted-2); font-size: 10px; letter-spacing: .08em; text-transform: uppercase; background: #0e141d; border-bottom: 1px solid var(--line-soft); }
  .table-row { padding: 12px 14px; border-bottom: 1px solid #1b222c; font-size: 12px; }
  .table-row:last-child { border-bottom: 0; }
  .table-row:hover { background: #151d28; }
  .disabled-row { opacity: .55; }
  .cell-title { font-weight: 600; display: flex; align-items: center; gap: 6px; }
  .cell-sub { color: var(--muted); font-size: 10.5px; margin-top: 3px; }
  .cell-error { color: #fca5a5; font-size: 10px; margin-top: 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 240px; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted); }
  .expression { color: #a8b4c8; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row-actions { display: flex; justify-content: flex-end; gap: 4px; }
  .right { text-align: right; }
  .badge.ok { color: #a7f3d0; border-color: #174b3e; }
  .badge.bad { color: #fecaca; border-color: #4a2529; }
  .badge.warn { color: #fde68a; border-color: #4b3a16; }
  .empty-state { min-height: 260px; }
  .empty-icon { font-size: 34px; color: #3c4b60; }
  .form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .form-grid.three { grid-template-columns: repeat(3, 1fr); }
  .quick-row { display: flex; align-items: center; gap: 5px; flex-wrap: wrap; margin: -4px 0 12px; }
  .preview-box { border: 1px solid var(--line-soft); background: #0f141d; border-radius: 9px; padding: 11px 12px; margin: 4px 0 14px; }
  .preview-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; font-size: 12px; }
  .preview-list { color: var(--accent); font-size: 11px; line-height: 1.6; }
  .check-row { display: flex; align-items: center; gap: 8px; color: var(--muted); font-size: 12px; }
  .check-row input { accent-color: var(--accent-strong); }
  @media (max-width: 1100px) {
    .schedule-grid { grid-template-columns: minmax(140px, 1fr) 110px minmax(140px, 1fr) 120px 100px 218px; }
  }
  @media (max-width: 900px) {
    .schedule-grid { grid-template-columns: minmax(160px, 1fr) 110px 120px 190px; }
    .table-row > div:nth-child(3), .table-head > span:nth-child(3) { display: none; }
    .form-grid, .form-grid.three { grid-template-columns: 1fr; }
  }
</style>
