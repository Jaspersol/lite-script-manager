<script lang="ts">
  import { onMount } from "svelte";
  import { api, formatTime, friendlyError } from "./api";
  import { scriptName, statusLabel } from "./format";
  import { createListenerGroup } from "./listeners";
  import Icon from "./Icon.svelte";
  import { pageAction } from "./uiActions";
  import type { PageAction } from "./uiActions";
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

  $effect(() => {
    const action: PageAction = {
      label: "新建计划",
      icon: "plus",
      kind: "primary",
      disabled: scripts.length === 0,
      run: openNew,
    };
    pageAction.set(action);
    return () => pageAction.set(null);
  });

  onMount(() => {
    load();
    const listeners = createListenerGroup();
    listeners.add("schedules-changed", load);
    return () => listeners.dispose();
  });
</script>

<div class="schedule-page">
  {#if loading}
    <div class="empty-state"><div class="spinner"></div>正在加载计划…</div>
  {:else if schedules.length === 0}
    <div class="empty-state">
      <div class="empty-icon"><Icon name="clock" size={34} strokeWidth={1.25} /></div>
      <h2>还没有计划任务</h2>
      <p>为脚本设置固定间隔、Cron 或后台循环重启。</p>
      <button class="btn btn-primary" onclick={openNew} disabled={scripts.length === 0}><Icon name="plus" size={15} /> 新建计划</button>
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
          <div class="ellipsis">{scriptName(scripts, schedule.script_id)}</div>
          <div class="mono expression">{schedule.expression}</div>
          <div class="cell-sub">{schedule.enabled ? formatTime(schedule.next_run_at) : "已禁用"}</div>
          <div>
            <span class="badge" class:ok={schedule.last_status === "running" || schedule.last_status === "exited"} class:bad={schedule.last_status === "failed"}>{statusLabel(schedule.last_status)}</span>
            {#if schedule.last_error}<div class="cell-error" title={schedule.last_error}>{schedule.last_error}</div>{/if}
          </div>
          <div class="row-actions">
            <button class="btn btn-sm" onclick={() => runNow(schedule)}>执行</button>
            <button class="btn btn-sm btn-ghost" onclick={() => toggle(schedule)}>{schedule.enabled ? "禁用" : "启用"}</button>
            <button class="icon-btn btn btn-ghost" title="编辑" aria-label="编辑计划" onclick={() => openEdit(schedule)}><Icon name="edit" size={15} /></button>
            <button class="icon-btn btn btn-ghost" title="删除" aria-label="删除计划" onclick={() => remove(schedule)}><Icon name="trash" size={15} /></button>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if edit}
  <div class="modal-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) edit = null; }}>
    <div class="modal wide" role="dialog" aria-modal="true" aria-label="编辑计划">
      <div class="modal-head">
        <div class="modal-title">{editing ? "编辑计划" : "新建计划"}</div>
        <button class="btn btn-ghost icon-btn" title="关闭" aria-label="关闭" onclick={() => { edit = null; editing = null; }}><Icon name="close" size={17} /></button>
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
  .schedule-page { container-type: inline-size; }
  .table-card { border: 1px solid var(--line-soft); border-radius: 16px; overflow: hidden; background: #fff; }
  .schedule-grid {
    display: grid; grid-template-columns: minmax(110px, 1.15fr) 78px minmax(95px, 1.2fr) 88px 70px 160px;
    gap: 6px; align-items: center;
  }
  .table-head {
    padding: 11px 16px; color: #8b929d; font-size: 11px; font-weight: 500;
    letter-spacing: .01em; background: #fafbfc; border-bottom: 1px solid var(--line-soft);
  }
  .table-row {
    padding: 13px 16px; border-bottom: 1px solid #f3f4f7; font-size: 12.5px;
    transition: background-color var(--dur-1) var(--ease);
  }
  .table-row:last-child { border-bottom: 0; }
  .table-row:hover { background: #fafbfd; }
  .disabled-row { opacity: .52; }
  .cell-title { font-weight: 550; display: flex; align-items: center; gap: 6px; color: #363c45; }
  .cell-sub { color: #8a919c; font-size: 11px; margin-top: 4px; }
  .cell-error { color: #b86666; font-size: 10.5px; margin-top: 5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 240px; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted); }
  .expression { color: #6f7a8d; font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row-actions { display: flex; justify-content: flex-end; gap: 2px; align-items: center; }
  .row-actions .btn:not(.btn-ghost) { background: #eef4fc; color: #4c6da9; }
  .right { text-align: right; }
  .empty-state { min-height: 280px; }
  .quick-row { display: flex; align-items: center; gap: 5px; flex-wrap: wrap; margin: -4px 0 13px; }
  .preview-box { border: 1px solid var(--line-soft); background: #fafbfc; border-radius: 12px; padding: 12px 13px; margin: 4px 0 15px; }
  .preview-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 7px; font-size: 12px; color: #4b525d; }
  .preview-list { color: #5d7bb8; font-size: 11px; line-height: 1.65; }
  @container (max-width: 665px) {
    .schedule-grid { grid-template-columns: minmax(115px, 1.1fr) 82px minmax(95px, 1.2fr) 90px 160px; }
    .table-row > :nth-child(5), .table-head > :nth-child(5) { display: none; }
  }
</style>
