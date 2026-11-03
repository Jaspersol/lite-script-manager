<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, formatTime, friendlyError } from "./api";
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
  let edit = $state<TriggerEdit | null>(null);
  let name = $state("");
  let enabled = $state(true);
  let eventType = $state("process_started");
  let cooldown = $state(0);
  let processName = $state("");
  let processPath = $state("");
  let processParent = $state("");
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

  function scriptName(id: string): string {
    return scripts.find((script) => script.id === id)?.name ?? "已删除脚本";
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
    command = "";
    metric = "system.cpu";
    op = ">=";
    value = 80;
    forSec = 10;
    hysteresis = 5;
    actions = [blankAction()];
    edit = { name: "", enabled: true, event_type: "process_started", config_json: "{}", actions_json: "[]", cooldown_sec: 30 };
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
      processName = config.name ?? "";
      processPath = config.path ?? "";
      processParent = config.parent ?? "";
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
    edit = {
      name: trigger.name,
      enabled: trigger.enabled,
      event_type: trigger.event_type,
      config_json: trigger.config_json,
      actions_json: trigger.actions_json,
      cooldown_sec: trigger.cooldown_sec,
    };
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
      return JSON.stringify({
        name: processName.trim(),
        path: processPath.trim(),
        metric,
        op,
        value: Number(value),
        for_sec: Number(forSec),
        hysteresis: Number(hysteresis),
      });
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
      notify("请填写触发器名称", "error");
      return;
    }
    if (actions.length === 0) {
      notify("至少需要一个动作", "error");
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
      edit = null;
      editing = null;
      await load();
      notify("触发器已保存");
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
    if (!confirm(`删除触发器「${trigger.name}」？`)) return;
    try {
      await api.deleteTrigger(trigger.id);
      await load();
      notify("触发器已删除");
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  async function fire(trigger: Trigger) {
    try {
      const result = await api.triggerNow(trigger.id);
      notify(`已触发：${result}`);
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
        return `${metricLabels[config.metric] ?? config.metric} ${config.op} ${config.value}，持续 ${config.for_sec ?? 0}s`;
      }
      const parts = [];
      if (config.name) parts.push(`名称 ${config.name}`);
      if (config.path) parts.push(`路径含 ${config.path}`);
      if (config.parent) parts.push(`父进程 ${config.parent}`);
      return parts.length ? parts.join(" · ") : "任意进程";
    } catch {
      return trigger.config_json;
    }
  }

  onMount(() => {
    load();
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    (async () => {
      const off = await listen("triggers-changed", load);
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
    <button class="btn btn-primary" onclick={openNew}>+ 新建触发器</button>
  </div>

  {#if loading}
    <div class="empty-state"><div class="spinner"></div>正在加载触发器…</div>
  {:else if triggers.length === 0}
    <div class="empty-state">
      <div class="empty-icon">◇</div>
      <h2>还没有触发器</h2>
      <p>例如：游戏进程启动后执行准备脚本，CPU 持续高于 80% 时通知。</p>
      <button class="btn btn-primary" onclick={openNew}>+ 新建触发器</button>
    </div>
  {:else}
    <div class="trigger-grid">
      {#each triggers as trigger (trigger.id)}
        <div class="trigger-card" class:disabled={!trigger.enabled}>
          <div class="trigger-head">
            <div>
              <div class="trigger-name">{trigger.name}</div>
              <div class="cell-sub">{eventLabels[trigger.event_type] ?? trigger.event_type}</div>
            </div>
            <span class="status-dot" class:running={trigger.enabled}></span>
          </div>
          <div class="trigger-rule mono">{describe(trigger)}</div>
          <div class="trigger-meta">
            <span>冷却 {trigger.cooldown_sec}s</span>
            <span>上次 {formatTime(trigger.last_triggered_at)}</span>
          </div>
          {#if trigger.last_result}<div class="cell-error" title={trigger.last_result}>{trigger.last_result}</div>{/if}
          <div class="trigger-actions">
            <button class="btn btn-sm" onclick={() => fire(trigger)}>立即触发</button>
            <button class="btn btn-sm" onclick={() => toggle(trigger)}>{trigger.enabled ? "禁用" : "启用"}</button>
            <button class="btn btn-sm btn-ghost" onclick={() => openEdit(trigger)}>编辑</button>
            <button class="icon-btn btn btn-ghost" title="删除" onclick={() => remove(trigger)}>×</button>
          </div>
        </div>
      {/each}
    </div>
  {/if}

  <div class="recent-head">
    <h3>最近触发记录</h3>
  </div>
  {#if events.length === 0}
    <div class="recent-empty">还没有触发记录。</div>
  {:else}
    <div class="table-card">
      <div class="table-head event-grid"><span>时间</span><span>触发器</span><span>事件</span><span>结果</span></div>
      {#each events as event (event.id)}
        <div class="table-row event-grid">
          <span class="cell-sub">{formatTime(event.fired_at)}</span>
          <span class="ellipsis">{event.trigger_name}</span>
          <span class="mono tiny ellipsis">{event.event}</span>
          <span class="ellipsis" title={event.result}>{event.result}</span>
        </div>
      {/each}
    </div>
  {/if}
</section>

{#if edit}
  <div class="modal-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) { edit = null; editing = null; } }}>
    <div class="modal wide" role="dialog" aria-modal="true" aria-label="编辑触发器">
      <div class="modal-head">
        <div class="modal-title">{editing ? "编辑触发器" : "新建触发器"}</div>
        <button class="btn btn-ghost icon-btn" onclick={() => { edit = null; editing = null; }}>×</button>
      </div>
      <div class="modal-body">
        <div class="form-grid">
          <div class="field">
            <label class="label" for="trigger-name">名称</label>
            <input id="trigger-name" class="input" bind:value={name} placeholder="例如：游戏启动" />
          </div>
          <div class="field">
            <label class="label" for="trigger-event">事件</label>
            <select id="trigger-event" class="select" bind:value={eventType}>
              {#each Object.entries(eventLabels) as [key, label] (key)}<option value={key}>{label}</option>{/each}
            </select>
          </div>
        </div>

        {#if processTypes()}
          <div class="form-section">进程匹配</div>
          <div class="form-grid">
            <div class="field"><label class="label" for="trigger-process-name">进程名</label><input id="trigger-process-name" class="input mono" bind:value={processName} placeholder="game.exe" /></div>
            <div class="field"><label class="label" for="trigger-process-parent">父进程名</label><input id="trigger-process-parent" class="input mono" bind:value={processParent} placeholder="explorer.exe（可选）" /></div>
          </div>
          <div class="form-grid">
            <div class="field"><label class="label" for="trigger-process-path">路径包含</label><input id="trigger-process-path" class="input mono" bind:value={processPath} placeholder="D:\\Games（可选）" /></div>
            <div class="field"><label class="label" for="trigger-command">命令行包含</label><input id="trigger-command" class="input mono" bind:value={command} placeholder="--server（可选）" /></div>
          </div>
          <p class="muted tiny">只配置进程名时不需要查询命令行；路径/命令行匹配会增加一次进程查询。</p>
        {/if}

        {#if isResourceType()}
          <div class="form-section">资源条件</div>
          <div class="form-grid three">
            <div class="field"><label class="label" for="trigger-metric">指标</label>
              <select id="trigger-metric" class="select" bind:value={metric}>
                {#each Object.entries(metricLabels) as [key, label] (key)}<option value={key}>{label}</option>{/each}
              </select>
            </div>
            <div class="field"><label class="label" for="trigger-op">比较</label>
              <select id="trigger-op" class="select" bind:value={op}>
                <option value=">=">≥</option><option value=">">&gt;</option><option value="<=">≤</option><option value="<">&lt;</option><option value="==">=</option>
              </select>
            </div>
            <div class="field"><label class="label" for="trigger-value">阈值</label><input id="trigger-value" class="input" type="number" bind:value={value} /></div>
          </div>
          <div class="form-grid three">
            <div class="field"><label class="label" for="trigger-for">持续秒数</label><input id="trigger-for" class="input" type="number" min="0" bind:value={forSec} /></div>
            <div class="field"><label class="label" for="trigger-hysteresis">滞后值</label><input id="trigger-hysteresis" class="input" type="number" min="0" bind:value={hysteresis} /></div>
            <div class="field"><label class="label" for="trigger-cooldown">冷却秒数</label><input id="trigger-cooldown" class="input" type="number" min="0" bind:value={cooldown} /></div>
          </div>
        {:else}
          <div class="field cooldown-field"><label class="label" for="trigger-cooldown-plain">冷却秒数</label><input id="trigger-cooldown-plain" class="input" type="number" min="0" bind:value={cooldown} /></div>
        {/if}

        <div class="form-section actions-head">
          <span>动作链</span>
          <button class="btn btn-sm" onclick={addAction}>+ 添加动作</button>
        </div>
        {#each actions as action, index (index)}
          <div class="action-row">
            <select class="select" bind:value={action.type}>
              {#each Object.entries(actionLabels) as [key, label] (key)}<option value={key}>{label}</option>{/each}
            </select>
            {#if ["run_script", "stop_script", "restart_script"].includes(action.type)}
              <select class="select" bind:value={action.script_id}>
                {#each scripts as script (script.id)}<option value={script.id}>{script.name}</option>{/each}
              </select>
            {:else if ["enable_trigger", "disable_trigger"].includes(action.type)}
              <select class="select" bind:value={action.trigger_id}>
                <option value="">选择触发器</option>
                {#each triggers as trigger (trigger.id)}<option value={trigger.id}>{trigger.name}</option>{/each}
              </select>
            {:else}
              <input class="input" bind:value={action.title} placeholder="通知标题" />
              <input class="input" bind:value={action.message} placeholder="通知内容" />
            {/if}
            <button class="icon-btn btn btn-ghost" title="移除动作" onclick={() => removeAction(index)}>×</button>
          </div>
        {/each}

        <label class="check-row"><input type="checkbox" bind:checked={enabled} /> 启用此触发器</label>
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
  .trigger-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(290px, 1fr)); gap: 10px; }
  .trigger-card { border: 1px solid var(--line-soft); background: #111720; border-radius: 11px; padding: 14px; min-width: 0; }
  .trigger-card.disabled { opacity: .55; }
  .trigger-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; }
  .trigger-name { font-size: 13px; font-weight: 650; }
  .cell-sub { color: var(--muted); font-size: 10.5px; margin-top: 3px; }
  .trigger-rule { color: #a8b4c8; font-size: 11px; margin: 12px 0 8px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .trigger-meta { display: flex; gap: 12px; color: var(--muted-2); font-size: 10px; }
  .cell-error { color: #fca5a5; font-size: 10px; margin-top: 6px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .trigger-actions { display: flex; gap: 4px; margin-top: 12px; }
  .recent-head { margin: 26px 0 10px; }
  .recent-head h3 { font-size: 13px; margin: 0; }
  .recent-empty { color: var(--muted); font-size: 11px; padding: 12px 0; }
  .table-card { border: 1px solid var(--line-soft); border-radius: 11px; overflow: hidden; background: #111720; }
  .table-head { padding: 9px 13px; color: var(--muted-2); font-size: 10px; letter-spacing: .08em; text-transform: uppercase; background: #0e141d; border-bottom: 1px solid var(--line-soft); }
  .table-row { padding: 9px 13px; border-bottom: 1px solid #1b222c; font-size: 11px; }
  .table-row:last-child { border-bottom: 0; }
  .event-grid { display: grid; grid-template-columns: 150px 160px minmax(160px, 1fr) minmax(220px, 1.5fr); gap: 12px; align-items: center; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .form-grid.three { grid-template-columns: repeat(3, 1fr); }
  .form-section { color: var(--muted); font-size: 11px; font-weight: 650; letter-spacing: .06em; text-transform: uppercase; border-top: 1px solid var(--line-soft); padding-top: 14px; margin: 4px 0 12px; }
  .actions-head { display: flex; align-items: center; justify-content: space-between; }
  .action-row { display: grid; grid-template-columns: 130px minmax(130px, 1fr) auto; gap: 6px; margin-bottom: 7px; align-items: center; }
  .action-row:has(input) { grid-template-columns: 130px minmax(100px, .8fr) minmax(130px, 1.4fr) auto; }
  .cooldown-field { max-width: 200px; }
  .check-row { display: flex; align-items: center; gap: 8px; color: var(--muted); font-size: 12px; margin-top: 8px; }
  .check-row input { accent-color: var(--accent-strong); }
  @media (max-width: 900px) {
    .form-grid, .form-grid.three { grid-template-columns: 1fr; }
    .event-grid { grid-template-columns: 110px 120px minmax(120px, 1fr); }
    .event-grid > :nth-child(4) { display: none; }
  }
</style>
