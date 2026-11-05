<script lang="ts">
  import { confirmDialog } from "./confirm";
  import { onMount } from "svelte";
  import { api, formatTime, friendlyError } from "./api";
  import { t, tBackend } from "./i18n.svelte";
  import { createListenerGroup } from "./listeners";
  import Icon from "./Icon.svelte";
  import Hint from "./Hint.svelte";
  import { pageAction } from "./uiActions";
  import type { PageAction } from "./uiActions";
  import type { Script, Trigger, TriggerEdit, TriggerEvent } from "./types";

  let {
    scripts = [],
    notify = () => {},
  }: {
    scripts: Script[];
    notify: (message: string, kind?: "info" | "error") => void;
  } = $props();

  interface ActionRow {
    type: string;
    script_id: string;
    trigger_id: string;
    title: string;
    message: string;
  }

  let triggers: Trigger[] = $state([]);
  let events: TriggerEvent[] = $state([]);
  let loading = $state(true);
  let editing: Trigger | null = $state(null);
  let editorOpen = $state(false);
  let originalConditions: unknown[] = $state([]);
  let name = $state("");
  let enabled = $state(true);
  let eventType = $state("process_started");
  let cooldown = $state(0);
  let processName = $state("");
  let processPath = $state("");
  let processParent = $state("");
  let scriptFilter = $state("");
  let command = $state("");
  let metric = $state("system.cpu");
  let op = $state(">=");
  let value = $state(80);
  let forSec = $state(10);
  let hysteresis = $state(5);
  let actions: ActionRow[] = $state([]);

  const eventLabels: Record<string, string> = {
    process_started: "进程启动",
    process_exited: "进程关闭",
    resource_enter: "资源进入阈值",
    resource_exit: "资源退出阈值",
    script_started: "受管脚本启动",
    script_exited: "受管脚本结束",
    manual: "手动触发",
  };
  const actionLabels: Record<string, string> = {
    run_script: "启动脚本",
    stop_script: "停止脚本",
    restart_script: "重启脚本",
    enable_trigger: "启用触发器",
    disable_trigger: "禁用触发器",
    notify: "系统通知",
  };
  const metricLabels: Record<string, string> = {
    "system.cpu": "系统 CPU %",
    "system.memory": "系统内存 %",
    "system.process_count": "进程数",
    "disk.free_gb": "磁盘剩余 GB",
    "process.memory": "匹配进程内存字节",
    "process.memory_mb": "匹配进程内存 MB",
    "process.cpu": "匹配进程 CPU %",
  };

  function isResourceType(type = eventType): boolean {
    return type === "resource_enter" || type === "resource_exit";
  }

  function processTypes(type = eventType): boolean {
    return type === "process_started" || type === "process_exited";
  }

  function scriptTypes(type = eventType): boolean {
    return type === "script_started" || type === "script_exited";
  }

  async function load() {
    try {
      triggers = await api.listTriggers();
      events = await api.listTriggerEvents(40);
    } catch (error) {
      notify(friendlyError(error), "error");
    } finally {
      loading = false;
    }
  }

  function blankAction(): ActionRow {
    return {
      type: "run_script",
      script_id: scripts[0]?.id ?? "",
      trigger_id: "",
      title: "",
      message: "",
    };
  }

  function openNew() {
    editing = null;
    name = "";
    enabled = true;
    eventType = "process_started";
    cooldown = 30;
    processName = "";
    processPath = "";
    processParent = "";
    scriptFilter = "";
    command = "";
    metric = "system.cpu";
    op = ">=";
    value = 80;
    forSec = 10;
    hysteresis = 5;
    actions = [blankAction()];
    originalConditions = [];
    editorOpen = true;
  }

  function openEdit(trigger: Trigger) {
    editing = trigger;
    name = trigger.name;
    enabled = trigger.enabled;
    eventType = trigger.event_type;
    cooldown = trigger.cooldown_sec;
    actions = [];
    try {
      const config = JSON.parse(trigger.config_json || "{}");
      originalConditions = Array.isArray(config.conditions) ? config.conditions : [];
      processName = config.name ?? "";
      processPath = config.path ?? "";
      processParent = config.parent ?? "";
      scriptFilter = config.script_id ?? "";
      command = config.command ?? "";
      metric = config.metric ?? config.conditions?.[0]?.metric ?? "system.cpu";
      op = config.op ?? config.conditions?.[0]?.op ?? ">=";
      value = config.value ?? config.conditions?.[0]?.value ?? 80;
      forSec = config.for_sec ?? config.conditions?.[0]?.for_sec ?? 10;
      hysteresis = config.hysteresis ?? config.conditions?.[0]?.hysteresis ?? 5;
    } catch {
      // Keep defaults for malformed JSON.
    }
    try {
      const parsed = JSON.parse(trigger.actions_json || "[]");
      if (Array.isArray(parsed)) {
        actions = parsed.map((item) => ({
          type: item.type ?? "notify",
          script_id: item.script_id ?? "",
          trigger_id: item.trigger_id ?? "",
          title: item.title ?? "",
          message: item.message ?? "",
        }));
      }
    } catch {
      // Keep empty actions.
    }
    if (actions.length === 0) actions = [blankAction()];
    editorOpen = true;
  }

  function buildConfig(): string {
    if (processTypes()) {
      return JSON.stringify({
        name: processName.trim(),
        path: processPath.trim(),
        parent: processParent.trim(),
        command: command.trim(),
      });
    }
    if (isResourceType()) {
      const condition = {
        metric,
        op,
        value: Number(value),
        for_sec: Number(forSec),
        hysteresis: Number(hysteresis),
      };
      const config: Record<string, unknown> = {
        name: processName.trim(),
        path: processPath.trim(),
        ...condition,
      };
      if (originalConditions.length > 1) {
        config.conditions = [condition, ...originalConditions.slice(1)];
      }
      return JSON.stringify(config);
    }
    if (scriptTypes()) {
      return JSON.stringify({ script_id: scriptFilter });
    }
    return "{}";
  }

  function buildActions(): string {
    return JSON.stringify(actions.map((action) => ({
      type: action.type,
      script_id: action.script_id || undefined,
      trigger_id: action.trigger_id || undefined,
      title: action.title || undefined,
      message: action.message || undefined,
    })));
  }

  async function save() {
    if (!name.trim()) {
      notify(t("触发器名称不能为空"), "error");
      return;
    }
    if (actions.length === 0) {
      notify(t("触发器至少需要一个动作"), "error");
      return;
    }
    const payload: TriggerEdit = {
      name: name.trim(),
      enabled,
      event_type: eventType,
      config_json: buildConfig(),
      actions_json: buildActions(),
      cooldown_sec: Number(cooldown),
    };
    try {
      if (editing) await api.updateTrigger(editing.id, payload);
      else await api.createTrigger(payload);
      editorOpen = false;
      editing = null;
      await load();
      notify(t("触发器已保存"));
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  function editFrom(trigger: Trigger): TriggerEdit {
    return {
      name: trigger.name,
      enabled: !trigger.enabled,
      event_type: trigger.event_type,
      config_json: trigger.config_json,
      actions_json: trigger.actions_json,
      cooldown_sec: trigger.cooldown_sec,
    };
  }

  async function toggle(trigger: Trigger) {
    try {
      await api.updateTrigger(trigger.id, editFrom(trigger));
      await load();
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  async function remove(trigger: Trigger) {
    const ok = await confirmDialog({
      title: t("删除触发器"),
      message: t("删除触发器「{name}」？", { name: trigger.name }),
      confirmLabel: t("删除"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteTrigger(trigger.id);
      await load();
      notify(t("触发器已删除"));
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  async function fire(trigger: Trigger) {
    try {
      const result = await api.triggerNow(trigger.id);
      notify(t("已触发：{result}", { result: tBackend(result) }));
      await load();
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  function addAction() {
    actions = [...actions, blankAction()];
  }

  function removeAction(index: number) {
    actions = actions.filter((_, position) => position !== index);
  }

  function describe(trigger: Trigger): string {
    try {
      const config = JSON.parse(trigger.config_json || "{}");
      if (isResourceType(trigger.event_type)) {
        return t("{metric} {op} {value}，持续 {seconds}s", { metric: t(metricLabels[config.metric] ?? config.metric), op: config.op, value: config.value, seconds: config.for_sec ?? 0 });
      }
      const parts = [];
      if (config.name) parts.push(t("名称 {name}", { name: config.name }));
      if (config.path) parts.push(t("路径含 {path}", { path: config.path }));
      if (config.parent) parts.push(t("父进程 {parent}", { parent: config.parent }));
      return parts.length ? parts.join(" · ") : t("任意进程");
    } catch {
      return trigger.config_json;
    }
  }

  $effect(() => {
    const action: PageAction = {
      label: t("新建触发器"),
      icon: "plus",
      kind: "primary",
      run: openNew,
    };
    pageAction.set(action);
    return () => pageAction.set(null);
  });

  onMount(() => {
    load();
    const listeners = createListenerGroup();
    listeners.add("triggers-changed", load);
    return () => listeners.dispose();
  });
</script>

<div>
  {#if loading}
    <div class="empty-state"><div class="spinner"></div>{t("正在加载触发器…")}</div>
  {:else if triggers.length === 0}
    <div class="empty-state">
      <div class="empty-icon"><Icon name="bolt" size={34} strokeWidth={1.25} /></div>
      <h2>{t("还没有触发器")}</h2>
      <p>{t("例如：游戏进程启动后执行准备脚本，CPU 持续高于 80% 时通知。")}</p>
      <button class="btn btn-primary" onclick={openNew}><Icon name="plus" size={15} /> {t("新建触发器")}</button>
    </div>
  {:else}
    <div class="trigger-grid">
      {#each triggers as trigger (trigger.id)}
        <div class="trigger-card" class:disabled={!trigger.enabled}>
          <div class="trigger-head">
            <div>
              <div class="trigger-name">{trigger.name}</div>
              <div class="cell-sub">{t(eventLabels[trigger.event_type] ?? trigger.event_type)}</div>
            </div>
            <span class="status-dot" class:running={trigger.enabled}></span>
          </div>
          <div class="trigger-rule mono">{describe(trigger)}</div>
          <div class="trigger-meta">
            <span>{t("冷却 {seconds}s", { seconds: trigger.cooldown_sec })}</span>
            <span>{t("上次 {time}", { time: formatTime(trigger.last_triggered_at) })}</span>
          </div>
          {#if trigger.last_result}<div class="cell-error" title={tBackend(trigger.last_result)}>{tBackend(trigger.last_result)}</div>{/if}
          <div class="trigger-actions">
            <button class="btn btn-sm" onclick={() => fire(trigger)}>{t("立即触发")}</button>
            <button class="btn btn-sm btn-ghost" onclick={() => toggle(trigger)}>{trigger.enabled ? t("禁用") : t("启用")}</button>
            <button class="icon-btn btn btn-ghost" title={t("编辑")} aria-label={t("编辑触发器")} onclick={() => openEdit(trigger)}><Icon name="edit" size={15} /></button>
            <button class="icon-btn btn btn-ghost" title={t("删除")} aria-label={t("删除触发器")} onclick={() => remove(trigger)}><Icon name="trash" size={15} /></button>
          </div>
        </div>
      {/each}
    </div>
  {/if}

  <div class="recent-head">
    <h3>{t("最近触发记录")}</h3>
  </div>
  {#if events.length === 0}
    <div class="recent-empty">{t("还没有触发记录。")}</div>
  {:else}
    <div class="table-card">
      <div class="table-head event-grid"><span>{t("时间")}</span><span>{t("触发器")}</span><span>{t("事件")}</span><span>{t("结果")}</span></div>
      {#each events as event (event.id)}
        <div class="table-row event-grid">
          <span class="cell-sub">{formatTime(event.fired_at)}</span>
          <span class="ellipsis">{event.trigger_name}</span>
          <span class="mono tiny ellipsis">{event.event}</span>
          <span class="ellipsis" title={tBackend(event.result)}>{tBackend(event.result)}</span>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if editorOpen}
  <div class="modal-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) { editorOpen = false; editing = null; } }}>
    <div class="modal wide" role="dialog" aria-modal="true" aria-label={t("编辑触发器")}>
      <div class="modal-head">
        <div class="modal-title">{editing ? t("编辑触发器") : t("新建触发器")}</div>
        <button class="btn btn-ghost icon-btn" title={t("关闭")} aria-label={t("关闭")} onclick={() => { editorOpen = false; editing = null; }}><Icon name="close" size={17} /></button>
      </div>
      <div class="modal-body">
        <div class="form-grid">
          <div class="field">
            <div class="label-row"><label class="label" for="trigger-name">{t("名称")}</label><Hint text={t("仅用于在触发器列表和通知中识别。")} /></div>
            <input id="trigger-name" class="input" bind:value={name} placeholder={t("例如：游戏启动")} />
          </div>
          <div class="field">
            <div class="label-row"><label class="label" for="trigger-event">{t("事件")}</label><Hint text={t("触发时机：进程启动/退出、CPU/内存等资源阈值，或脚本自身的运行生命周期事件。")} /></div>
            <select id="trigger-event" class="select" bind:value={eventType}>
              {#each Object.entries(eventLabels) as [key, label] (key)}<option value={key}>{t(label)}</option>{/each}
            </select>
          </div>
        </div>

        {#if processTypes()}
          <div class="form-section">{t("进程匹配")}<Hint text={t("只配置进程名时不需要查询命令行；路径/命令行匹配会增加一次进程查询。")} /></div>
          <div class="form-grid">
            <div class="field"><div class="label-row"><label class="label" for="trigger-process-name">{t("进程名")}</label><Hint text={t("要监听的进程文件名，例如 game.exe。")} /></div><input id="trigger-process-name" class="input mono" bind:value={processName} placeholder="game.exe" /></div>
            <div class="field"><div class="label-row"><label class="label" for="trigger-process-parent">{t("父进程名")}</label><Hint text={t("可选。只在指定父进程（如 explorer.exe）拉起的进程匹配时触发，用于排除安装器等临时进程。")} /></div><input id="trigger-process-parent" class="input mono" bind:value={processParent} placeholder={t("explorer.exe（可选）")} /></div>
          </div>
          <div class="form-grid">
            <div class="field"><div class="label-row"><label class="label" for="trigger-process-path">{t("路径包含")}</label><Hint text={t("可选。进程可执行文件路径包含该字符串时才触发，用于区分同名进程。")} /></div><input id="trigger-process-path" class="input mono" bind:value={processPath} placeholder={t("D:\\\\Games（可选）")} /></div>
            <div class="field"><div class="label-row"><label class="label" for="trigger-command">{t("命令行包含")}</label><Hint text={t("可选。进程命令行包含该字符串时才触发。")} /></div><input id="trigger-command" class="input mono" bind:value={command} placeholder={t("--server（可选）")} /></div>
          </div>
        {/if}

        {#if scriptTypes()}
          <div class="form-section">{t("脚本匹配")}</div>
          <div class="form-grid">
            <div class="field"><div class="label-row"><label class="label" for="trigger-script">{t("监听脚本")}</label><Hint text={t("留空则匹配所有受管脚本。由本触发器自身启动的运行不会再次触发它。")} /></div>
              <select id="trigger-script" class="select" bind:value={scriptFilter}>
                <option value="">{t("任意脚本")}</option>
                {#each scripts as item (item.id)}<option value={item.id}>{item.name}</option>{/each}
              </select>
            </div>
          </div>
        {/if}

        {#if isResourceType()}
          <div class="form-section">{t("资源条件")}<Hint text={t("按采样间隔读取一次资源指标，满足条件并持续指定秒数后才触发动作链。")} /></div>
          <div class="form-grid three">
            <div class="field"><div class="label-row"><label class="label" for="trigger-metric">{t("指标")}</label><Hint text={t("参与比较的资源指标，例如 CPU 使用率、内存占用或进程数。")} /></div>
              <select id="trigger-metric" class="select" bind:value={metric}>
                {#each Object.entries(metricLabels) as [key, label] (key)}<option value={key}>{t(label)}</option>{/each}
              </select>
            </div>
            <div class="field"><div class="label-row"><label class="label" for="trigger-op">{t("比较")}</label><Hint text={t("指标与阈值之间的比较方式，如 ≥、>、≤、< 或 =。")} /></div>
              <select id="trigger-op" class="select" bind:value={op}>
                <option value=">=">≥</option><option value=">">&gt;</option><option value="<=">≤</option><option value="<">&lt;</option><option value="==">=</option>
              </select>
            </div>
            <div class="field"><div class="label-row"><label class="label" for="trigger-value">{t("阈值")}</label><Hint text={t("达到该数值并满足持续秒数后触发动作链。CPU 按百分比填写。")} /></div><input id="trigger-value" class="input" type="number" bind:value={value} /></div>
          </div>
          <div class="form-grid three">
            <div class="field"><div class="label-row"><label class="label" for="trigger-for">{t("持续秒数")}</label><Hint text={t("条件需连续满足该秒数才触发，用于过滤瞬时尖峰；填 0 表示条件成立立即触发。")} /></div><input id="trigger-for" class="input" type="number" min="0" bind:value={forSec} /></div>
            <div class="field"><div class="label-row"><label class="label" for="trigger-hysteresis">{t("滞后值")}</label><Hint text={t("指标回落到阈值减去该值以下时才复位，避免在阈值附近抖动导致反复触发。")} /></div><input id="trigger-hysteresis" class="input" type="number" min="0" bind:value={hysteresis} /></div>
            <div class="field"><div class="label-row"><label class="label" for="trigger-cooldown">{t("冷却秒数")}</label><Hint text={t("一次触发后在该秒数内不再重复触发，0 表示不限制。")} /></div><input id="trigger-cooldown" class="input" type="number" min="0" bind:value={cooldown} /></div>
          </div>
        {:else}
          <div class="field cooldown-field"><div class="label-row"><label class="label" for="trigger-cooldown-plain">{t("冷却秒数")}</label><Hint text={t("一次触发后在该秒数内不再重复触发，0 表示不限制。")} /></div><input id="trigger-cooldown-plain" class="input" type="number" min="0" bind:value={cooldown} /></div>
        {/if}

        <div class="form-section actions-head">
          <span>{t("动作链")}<Hint text={t("触发后按顺序执行的动作，可组合启动、停止、重启脚本，发送通知或启停其他触发器。")} /></span>
          <button class="btn btn-sm" onclick={addAction}>{t("+ 添加动作")}</button>
        </div>
        {#each actions as action, index (index)}
          <div class="action-row">
            <select class="select" bind:value={action.type}>
              {#each Object.entries(actionLabels) as [key, label] (key)}<option value={key}>{t(label)}</option>{/each}
            </select>
            {#if ["run_script", "stop_script", "restart_script"].includes(action.type)}
              <select class="select" bind:value={action.script_id}>
                {#each scripts as script (script.id)}<option value={script.id}>{script.name}</option>{/each}
              </select>
            {:else if ["enable_trigger", "disable_trigger"].includes(action.type)}
              <select class="select" bind:value={action.trigger_id}>
                <option value="">{t("选择触发器")}</option>
                {#each triggers as trigger (trigger.id)}<option value={trigger.id}>{trigger.name}</option>{/each}
              </select>
            {:else}
              <input class="input" bind:value={action.title} placeholder={t("通知标题")} />
              <input class="input" bind:value={action.message} placeholder={t("通知内容")} />
            {/if}
            <button class="icon-btn btn btn-ghost" title={t("移除动作")} aria-label={t("移除动作")} onclick={() => removeAction(index)}><Icon name="close" size={14} /></button>
          </div>
        {/each}

        <label class="check-row"><input type="checkbox" bind:checked={enabled} /> <span class="check-text">{t("启用此触发器")}<Hint text={t("启用后后台按采样间隔监听该事件；禁用时保留配置但不监听。")} /></span></label>
      </div>
      <div class="modal-foot">
        <button class="btn" onclick={() => { editorOpen = false; editing = null; }}>{t("取消")}</button>
        <button class="btn btn-primary" onclick={save}>{t("保存")}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .trigger-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(290px, 1fr)); gap: 12px; }
  .trigger-card {
    border: 1px solid var(--line-soft); background: #fff; border-radius: 16px; padding: 16px;
    min-width: 0; transition: border-color var(--dur-2) var(--ease), background-color var(--dur-2) var(--ease), box-shadow var(--dur-2) var(--ease);
  }
  .trigger-card:hover { border-color: #e1e6ef; background: #fdfdff; box-shadow: var(--shadow-soft); }
  .trigger-card.disabled { opacity: .55; }
  .trigger-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; }
  .trigger-name { font-size: 13.5px; font-weight: 550; color: #363c45; }
  .cell-sub { color: #8a919c; font-size: 11px; margin-top: 4px; }
  .trigger-rule { color: #6f7a8d; font-size: 11.5px; margin: 14px 0 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .trigger-meta { display: flex; gap: 12px; color: var(--faint); font-size: 10.5px; }
  .cell-error { color: #b86666; font-size: 10.5px; margin-top: 7px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .trigger-actions { display: flex; gap: 2px; margin-top: 14px; align-items: center; }
  .trigger-actions .btn:not(.btn-ghost) { background: #eef4fc; color: #4c6da9; }
  .recent-head { margin: 28px 0 11px; }
  .recent-head h3 { font-size: 13px; margin: 0; font-weight: 550; color: #40474f; }
  .recent-empty { color: var(--faint); font-size: 11px; padding: 13px 0; }
  .table-card { border: 1px solid var(--line-soft); border-radius: 16px; overflow: hidden; background: #fff; }
  .table-head {
    padding: 11px 16px; color: #8b929d; font-size: 11px; font-weight: 500;
    letter-spacing: .01em; background: #fafbfc; border-bottom: 1px solid var(--line-soft);
  }
  .table-row { padding: 12px 16px; border-bottom: 1px solid #f3f4f7; font-size: 11px; color: var(--muted); }
  .table-row:last-child { border-bottom: 0; }
  .event-grid { display: grid; grid-template-columns: 130px 130px minmax(140px, 1fr) minmax(180px, 1.4fr); gap: 12px; align-items: center; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .form-section {
    display: flex; align-items: center; gap: 5px;
    color: #8b929d; font-size: 11px; font-weight: 600; letter-spacing: .02em;
    border-top: 1px solid var(--line-soft); padding-top: 16px; margin: 6px 0 13px;
  }
  .actions-head { display: flex; align-items: center; justify-content: space-between; }
  .action-row { display: grid; grid-template-columns: 130px minmax(130px, 1fr) auto; gap: 7px; margin-bottom: 8px; align-items: center; }
  .action-row:has(input) { grid-template-columns: 130px minmax(100px, .8fr) minmax(130px, 1.4fr) auto; }
  .cooldown-field { max-width: 200px; }
  @media (max-width: 900px) {
    .event-grid { grid-template-columns: 110px 120px minmax(120px, 1fr); }
    .event-grid > :nth-child(4) { display: none; }
  }
</style>
