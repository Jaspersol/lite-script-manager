<script lang="ts">
  import { confirmDialog } from "./confirm";
  import { onMount } from "svelte";
  import { api, formatTime, friendlyError } from "./api";
  import { scriptName, statusLabel } from "./format";
  import { t, tBackend } from "./i18n.svelte";
  import { createListenerGroup } from "./listeners";
  import Icon from "./Icon.svelte";
  import Hint from "./Hint.svelte";
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
      notify(t("计划名称不能为空"), "error");
      return;
    }
    if (!edit.script_id) {
      notify(t("请选择计划关联的脚本"), "error");
      return;
    }
    try {
      if (editing) await api.updateSchedule(editing.id, edit);
      else await api.createSchedule(edit);
      edit = null;
      editing = null;
      await load();
      notify(t("计划已保存"));
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  async function remove(schedule: Schedule) {
    const ok = await confirmDialog({
      title: t("删除计划"),
      message: t("删除计划「{name}」？", { name: schedule.name }),
      confirmLabel: t("删除"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteSchedule(schedule.id);
      await load();
      notify(t("计划已删除"));
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
      notify(t("已触发「{name}」", { name: schedule.name }));
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
      label: t("新建计划"),
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
    <div class="empty-state"><div class="spinner"></div>{t("正在加载计划…")}</div>
  {:else if schedules.length === 0}
    <div class="empty-state">
      <div class="empty-icon"><Icon name="clock" size={34} strokeWidth={1.25} /></div>
      <h2>{t("还没有计划任务")}</h2>
      <p>{t("为脚本设置固定间隔、Cron 或后台循环重启。")}</p>
      <button class="btn btn-primary" onclick={openNew} disabled={scripts.length === 0}><Icon name="plus" size={15} /> {t("新建计划")}</button>
    </div>
  {:else}
    <div class="table-card">
      <div class="table-head schedule-grid">
        <span>{t("计划")}</span><span>{t("脚本")}</span><span>{t("规则")}</span><span>{t("下次运行")}</span><span>{t("上次结果")}</span><span class="right">{t("操作")}</span>
      </div>
      {#each schedules as schedule (schedule.id)}
        <div class="table-row schedule-grid" class:disabled-row={!schedule.enabled}>
          <div class="cell-main">
            <div class="cell-title">
              {schedule.name}
              {#if conflicts.has(schedule.script_id)}<span class="badge warn" title={t("同一脚本存在多个启用计划")}>{t("冲突")}</span>{/if}
            </div>
            <div class="cell-sub">{t(kindLabels[schedule.kind] ?? schedule.kind)} · {schedule.timezone}</div>
          </div>
          <div class="ellipsis">{scriptName(scripts, schedule.script_id)}</div>
          <div class="mono expression">{schedule.expression}</div>
          <div class="cell-sub">{schedule.enabled ? formatTime(schedule.next_run_at) : t("已禁用")}</div>
          <div>
            <span class="badge" class:ok={schedule.last_status === "running" || schedule.last_status === "exited"} class:bad={schedule.last_status === "failed"}>{statusLabel(schedule.last_status)}</span>
            {#if schedule.last_error}<div class="cell-error" title={tBackend(schedule.last_error)}>{tBackend(schedule.last_error)}</div>{/if}
          </div>
          <div class="row-actions">
            <button class="btn btn-sm" onclick={() => runNow(schedule)}>{t("执行")}</button>
            <button class="btn btn-sm btn-ghost" onclick={() => toggle(schedule)}>{schedule.enabled ? t("禁用") : t("启用")}</button>
            <button class="icon-btn btn btn-ghost" title={t("编辑")} aria-label={t("编辑计划")} onclick={() => openEdit(schedule)}><Icon name="edit" size={15} /></button>
            <button class="icon-btn btn btn-ghost" title={t("删除")} aria-label={t("删除计划")} onclick={() => remove(schedule)}><Icon name="trash" size={15} /></button>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if edit}
  <div class="modal-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) edit = null; }}>
    <div class="modal wide" role="dialog" aria-modal="true" aria-label={t("编辑计划")}>
      <div class="modal-head">
        <div class="modal-title">{editing ? t("编辑计划") : t("新建计划")}</div>
        <button class="btn btn-ghost icon-btn" title={t("关闭")} aria-label={t("关闭")} onclick={() => { edit = null; editing = null; }}><Icon name="close" size={17} /></button>
      </div>
      <div class="modal-body">
        <div class="form-grid">
          <div class="field">
            <div class="label-row"><label class="label" for="schedule-name">{t("名称")}</label><Hint text={t("仅用于在计划列表中识别，不影响执行。")} /></div>
            <input id="schedule-name" class="input" bind:value={edit.name} placeholder={t("例如：每日备份")} />
          </div>
          <div class="field">
            <div class="label-row"><label class="label" for="schedule-script">{t("脚本")}</label><Hint text={t("该计划到点后启动的脚本；同一脚本存在多个启用计划时会在列表中标为冲突。")} /></div>
            <select id="schedule-script" class="select" bind:value={edit.script_id}>
              {#each scripts as script (script.id)}<option value={script.id}>{script.name}</option>{/each}
            </select>
          </div>
        </div>

        <div class="form-grid">
          <div class="field">
            <div class="label-row"><label class="label" for="schedule-kind">{t("类型")}</label><Hint text={t("Cron 按日历时间触发；固定间隔按周期性触发；一次性只运行一次；退出后循环在脚本退出后按间隔重启。")} /></div>
            <select id="schedule-kind" class="select" bind:value={edit.kind}>
              <option value="cron">Cron</option>
              <option value="interval">{t("固定间隔")}</option>
              <option value="once">{t("一次性")}</option>
              <option value="loop">{t("退出后循环重启")}</option>
            </select>
          </div>
          <div class="field">
            <div class="label-row"><label class="label" for="schedule-timezone">{t("时区")}</label><Hint text={t("Cron 与一次性计划使用该时区计算时间，可填 local、UTC 或 Asia/Shanghai 等时区名。")} /></div>
            <input id="schedule-timezone" class="input mono" bind:value={edit.timezone} placeholder="local / UTC / Asia/Shanghai" />
          </div>
        </div>

        <div class="field">
          <div class="label-row"><label class="label" for="schedule-expression">
            {edit.kind === "cron" ? t("Cron 表达式（5 段或 6 段）") : edit.kind === "interval" ? t("间隔（如 30s、5m、1h）") : edit.kind === "once" ? t("执行时间（2026-01-01 12:00 或 RFC3339）") : t("循环间隔（如 10s、5m）")}
          </label><Hint text={edit.kind === "cron" ? t("5 段格式：分 时 日 月 周；6 段格式在最前再加秒，例如 0 9 * * 1-5。") : edit.kind === "interval" ? t("按上一次触发时间起算的固定间隔，可写 30s、5m、1h。") : edit.kind === "once" ? t("本地时间写 2026-01-01 12:00，或使用 RFC3339 时间；错过会在下次启动时按“错过后”策略处理。") : t("脚本每次退出后等待该间隔再重新启动。")} /></div>
          <input id="schedule-expression" class="input mono" bind:value={edit.expression} />
        </div>
        {#if edit.kind === "cron"}
          <div class="quick-row">
            <span class="tiny muted">{t("快捷：")}</span>
            <button class="btn btn-sm" onclick={() => applyQuick("0 9 * * *")}>{t("每天 09:00")}</button>
            <button class="btn btn-sm" onclick={() => applyQuick("0 9 * * 1")}>{t("每周一 09:00")}</button>
            <button class="btn btn-sm" onclick={() => applyQuick("0 * * * *")}>{t("每小时")}</button>
            <button class="btn btn-sm" onclick={() => applyQuick("*/5 * * * *")}>{t("每 5 分钟")}</button>
          </div>
        {/if}

        <div class="preview-box">
          <div class="preview-head">
            <div class="label-row"><strong>{t("下次执行预览")}</strong><Hint text={t("按当前表达式计算接下来几次触发时间，用于验证表达式是否正确。")} /></div>
            <button class="btn btn-sm" onclick={updatePreview} disabled={edit.kind === "loop"}>{t("计算")}</button>
          </div>
          {#if previewError}<div class="cell-error">{previewError}</div>{/if}
          {#if preview.length}
            <div class="preview-list">{preview.map((value) => formatTime(value)).join("  ·  ")}</div>
          {:else if !previewError}
            <div class="tiny muted">{edit.kind === "loop" ? t("循环计划在每次退出后按间隔重新启动。") : t("点击“计算”验证表达式。")}</div>
          {/if}
        </div>

        <div class="form-grid">
          <div class="field">
            <div class="label-row"><label class="label" for="schedule-overlap">{t("重叠策略")}</label><Hint text={t("到点时脚本仍在运行的取舍：跳过本次、排队稍后重试、停止旧实例后启动，或允许并行。")} /></div>
            <select id="schedule-overlap" class="select" bind:value={edit.overlap_policy}>
              <option value="skip">{t("跳过")}</option>
              <option value="queue">{t("排队（延迟重试）")}</option>
              <option value="replace">{t("停止旧实例后启动")}</option>
              <option value="parallel">{t("允许并行")}</option>
            </select>
          </div>
          <div class="field">
            <div class="label-row"><label class="label" for="schedule-catchup">{t("错过后")}</label><Hint text={t("应用未运行或休眠错过触发时间后的补偿方式：不补、补一次，或补全部（最多 10 次）。")} /></div>
            <select id="schedule-catchup" class="select" bind:value={edit.catch_up}>
              <option value="none">{t("不补")}</option>
              <option value="once">{t("补一次")}</option>
              <option value="all">{t("补全部（最多 10 次）")}</option>
            </select>
          </div>
        </div>

        <div class="form-grid three">
          <div class="field">
            <div class="label-row"><label class="label" for="schedule-timeout">{t("最大运行秒数")}</label><Hint text={t("运行超过该时长自动结束并记为失败；留空则使用脚本自身的设置。")} /></div>
            <input id="schedule-timeout" class="input" type="number" min="0" placeholder={t("留空使用脚本设置")} bind:value={edit.timeout_sec} />
          </div>
          <div class="field">
            <div class="label-row"><label class="label" for="schedule-success">{t("成功退出码")}</label><Hint text={t("该计划判定成功的退出码，多个用逗号分隔并支持区间，例如 0,2,10-12。")} /></div>
            <input id="schedule-success" class="input mono" bind:value={edit.success_codes} placeholder={t("0 或 0,2,10-12")} />
          </div>
          <div class="field">
            <div class="label-row"><label class="label" for="schedule-restarts">{t("最大重启次数")}</label><Hint text={t("失败后自动重启的次数上限，0 表示不自动重启。")} /></div>
            <input id="schedule-restarts" class="input" type="number" min="0" bind:value={edit.max_restarts} />
          </div>
        </div>

        {#if edit.kind === "loop"}
          <div class="form-grid">
            <div class="field">
              <div class="label-row"><label class="label" for="schedule-delay">{t("退出后延迟秒数")}</label><Hint text={t("脚本退出后等待多久再重新启动，避免短时间内反复拉起。")} /></div>
              <input id="schedule-delay" class="input" type="number" min="0" bind:value={edit.restart_delay_sec} />
            </div>
            <label class="check-row"><input type="checkbox" bind:checked={edit.backoff} /> <span class="check-text">{t("失败时使用 1/2/5/10/30 秒指数退避")}<Hint text={t("连续失败时逐级拉长重启间隔，成功运行后重置。")} /></span></label>
          </div>
        {/if}

        <label class="check-row"><input type="checkbox" bind:checked={edit.enabled} /> <span class="check-text">{t("启用此计划")}<Hint text={t("启用后按计划自动执行；禁用时保留配置但不触发。")} /></span></label>
      </div>
      <div class="modal-foot">
        <button class="btn" onclick={() => { edit = null; editing = null; }}>{t("取消")}</button>
        <button class="btn btn-primary" onclick={save}>{t("保存")}</button>
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
  .preview-head .label-row { margin-bottom: 0; font-size: 12px; color: #4b525d; }
  .preview-list { color: #5d7bb8; font-size: 11px; line-height: 1.65; }
  @container (max-width: 665px) {
    .schedule-grid { grid-template-columns: minmax(115px, 1.1fr) 82px minmax(95px, 1.2fr) 90px 160px; }
    .table-row > :nth-child(5), .table-head > :nth-child(5) { display: none; }
  }
</style>
