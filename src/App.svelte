<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, friendlyError } from "./lib/api";
  import Schedules from "./lib/Schedules.svelte";
  import Triggers from "./lib/Triggers.svelte";
  import History from "./lib/History.svelte";
  import SettingsPage from "./lib/SettingsPage.svelte";
  import Icon from "./lib/Icon.svelte";
  import { pageAction } from "./lib/uiActions";
  import type { RunStateEvent, RuntimeInfo, RuntimeMeta, Script, ScriptEdit, Settings } from "./lib/types";

  type Tab = "scripts" | "schedules" | "triggers" | "history" | "runtimes" | "settings";

  let activeTab: Tab = $state("scripts");
  let scripts: Script[] = $state([]);
  let runtimes: RuntimeInfo[] = $state([]);
  let catalog: RuntimeMeta[] = $state([]);
  let loading = $state(true);
  let runtimeLoading = $state(false);
  let search = $state("");
  let toast = $state<{ message: string; kind: "info" | "error" } | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  let showAdd = $state(false);
  let addMode: "path" | "new" = $state("path");
  let selectedPath = $state("");
  let newName = $state("");
  let newRuntime = $state("python");
  let newContent = $state('print("Hello from LiteScriptManager")');

  let editScript: Script | null = $state(null);
  let editArgsText = $state("");
  let editInterpreterPath = $state("");
  let editProcessMatch = $state("");
  let dragId: string | null = $state(null);
  let firstRun = $state(false);
  let firstRunSaving = $state(false);

  const scriptExtensions = ["py", "js", "mjs", "cjs", "sh", "bash", "pl", "php", "R", "r", "lua", "bat", "cmd", "ps1", "vbs", "vbe", "wsf", "ahk", "au3"];

  const filteredScripts = $derived(
    scripts.filter((script) => {
      const term = search.trim().toLowerCase();
      if (!term) return true;
      return `${script.name} ${script.path} ${script.runtime_id}`.toLowerCase().includes(term);
    }),
  );

  const runningCount = $derived(scripts.filter((s) => s.status === "running" || s.status === "starting").length);
  const tabTitle = $derived({
    scripts: "脚本",
    schedules: "计划任务",
    triggers: "事件触发器",
    history: "运行历史",
    runtimes: "运行时环境",
    settings: "设置",
  }[activeTab]);
  const tabSubtitle = $derived({
    scripts: "管理本机脚本，按顺序显示在托盘快捷面板中。",
    schedules: "Cron、固定间隔、一次性执行和退出后循环重启。",
    triggers: "进程快照差分、资源阈值、脚本生命周期和手动触发；进程轮询默认 1 秒。",
    history: "运行结果、退出码、耗时和日志尾部。",
    runtimes: "只检测，不安装；检测结果用于运行前预检。",
    settings: "应用、窗口、日志、监控与安全设置。",
  }[activeTab]);

  function runtimeName(id: string): string {
    return catalog.find((item) => item.id === id)?.name ?? id;
  }

  function displayPath(path: string): string {
    return path.replace(/^\\\\\?\\/, "");
  }

  function isBusy(script: Script): boolean {
    return script.status === "starting" || script.status === "stopping" || script.status === "queued";
  }

  function statusLabel(status: string): string {
    return {
      stopped: "已停止",
      queued: "排队中",
      starting: "启动中",
      running: "运行中",
      stopping: "停止中",
      exited: "已结束",
      failed: "失败",
    }[status] ?? status;
  }

  function templateFor(runtimeId: string): string {
    switch (runtimeId) {
      case "python": return 'print("Hello from LiteScriptManager")';
      case "javascript": return 'console.log("Hello from LiteScriptManager");';
      case "powershell": return 'Write-Host "Hello from LiteScriptManager"';
      case "batch": return '@echo off\r\necho Hello from LiteScriptManager';
      case "shell": return '#!/usr/bin/env bash\necho "Hello from LiteScriptManager"';
      case "perl": return 'print "Hello from LiteScriptManager\\n";';
      case "php": return '<?php\necho "Hello from LiteScriptManager\\n";';
      case "r": return 'cat("Hello from LiteScriptManager\\n")';
      case "lua": return 'print("Hello from LiteScriptManager")';
      case "vbscript": return 'WScript.Echo "Hello from LiteScriptManager"';
      case "jscript": return 'WScript.Echo("Hello from LiteScriptManager");';
      case "wsf": return '<job id="hello">\n<script language="VBScript">\nWScript.Echo "Hello from LiteScriptManager"\n<\\/script>\n</job>';
      case "autohotkey": return 'MsgBox "Hello from LiteScriptManager"';
      case "autoit": return 'MsgBox(0, "LiteScriptManager", "Hello from LiteScriptManager")';
      default: return "";
    }
  }

  function showToast(message: string, kind: "info" | "error" = "info") {
    toast = { message, kind };
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { toast = null; }, 4200);
  }

  async function refreshScripts(showError = false) {
    try {
      scripts = await api.listScripts();
    } catch (error) {
      if (showError) showToast(friendlyError(error), "error");
    }
  }

  async function loadCatalog() {
    try {
      catalog = await api.listRuntimeCatalog();
    } catch (error) {
      showToast(friendlyError(error), "error");
    }
  }

  async function loadRuntimes(force = false) {
    runtimeLoading = true;
    try {
      runtimes = await api.detectRuntimes(force);
    } catch (error) {
      showToast(friendlyError(error), "error");
    } finally {
      runtimeLoading = false;
    }
  }

  async function adoptExternalRuns() {
    try {
      const count = await api.adoptExternalRuns();
      if (count > 0) {
        await refreshScripts();
        showToast(`已接管 ${count} 个运行中的脚本`);
      }
    } catch (error) {
      showToast(friendlyError(error), "error");
    }
  }

  async function toggleScript(script: Script) {
    if (isBusy(script)) return;
    const nextStatus = script.status === "running" ? "stopping" : "starting";
    scripts = scripts.map((item) => item.id === script.id ? { ...item, status: nextStatus } : item);
    try {
      if (script.status === "running") {
        await api.stopScript(script.id);
      } else {
        await api.runScript(script.id);
      }
    } catch (error) {
      scripts = scripts.map((item) => item.id === script.id ? { ...item, status: "failed", last_error: friendlyError(error) } : item);
      showToast(friendlyError(error), "error");
    }
  }

  function openAddPath() {
    addMode = "path";
    selectedPath = "";
    showAdd = true;
  }

  async function choosePath() {
    try {
      const result = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "脚本文件", extensions: scriptExtensions }],
      });
      if (typeof result === "string") selectedPath = result;
    } catch (error) {
      showToast(friendlyError(error), "error");
    }
  }

  async function addFromPath() {
    if (!selectedPath) return;
    try {
      await api.createFromPath(selectedPath);
      showAdd = false;
      selectedPath = "";
      await refreshScripts();
      showToast("脚本已添加");
    } catch (error) {
      showToast(friendlyError(error), "error");
    }
  }

  async function createNewScript() {
    if (!newName.trim()) {
      showToast("请填写脚本名称", "error");
      return;
    }
    try {
      await api.createNew(newName.trim(), newRuntime, newContent);
      showAdd = false;
      newName = "";
      await refreshScripts();
      showToast("脚本已创建");
    } catch (error) {
      showToast(friendlyError(error), "error");
    }
  }

  function openEdit(script: Script) {
    editScript = { ...script, args: [...script.args] };
    editArgsText = script.args.join("\n");
    editInterpreterPath = script.interpreter_path ?? "";
    editProcessMatch = script.process_match ?? "";
  }

  async function saveEdit() {
    if (!editScript) return;
    try {
      const edit: ScriptEdit = {
        name: editScript.name,
        args: editArgsText.split("\n").map((arg) => arg.trim()).filter((arg) => arg.length > 0),
        cwd: editScript.cwd || null,
        interpreter_path: editInterpreterPath.trim() || null,
        process_match: editProcessMatch.trim() || null,
        stop_timeout_sec: editScript.stop_timeout_sec || 3,
        timeout_sec: editScript.timeout_sec ? Number(editScript.timeout_sec) : null,
        stop_mode: editScript.stop_mode || "graceful",
        max_instances: editScript.max_instances || 1,
        instance_policy: editScript.instance_policy || "parallel",
        success_codes: editScript.success_codes || "0",
        enabled: editScript.enabled,
      };
      await api.updateScript(editScript.id, edit);
      editScript = null;
      await refreshScripts();
      showToast("脚本配置已保存");
    } catch (error) {
      showToast(friendlyError(error), "error");
    }
  }

  async function toggleHashPin() {
    if (!editScript) return;
    try {
      const hash = await api.pinScriptHash(editScript.id, !editScript.content_hash);
      editScript = { ...editScript, content_hash: hash };
      await refreshScripts();
      showToast(hash ? "已固定脚本内容哈希" : "已取消哈希固定");
    } catch (error) {
      showToast(friendlyError(error), "error");
    }
  }

  async function dismissFirstRun() {
    firstRunSaving = true;
    try {
      await api.updateSettings({ first_run_done: "true" });
      firstRun = false;
    } catch (error) {
      showToast(friendlyError(error), "error");
    } finally {
      firstRunSaving = false;
    }
  }

  async function deleteScript(script: Script) {
    if (!confirm(`从列表中移除「${script.name}」？\n默认不会删除磁盘上的脚本文件。`)) return;
    try {
      await api.deleteScript(script.id, false);
      await refreshScripts();
      showToast("已从列表移除");
    } catch (error) {
      showToast(friendlyError(error), "error");
    }
  }

  function onDragStart(event: DragEvent, id: string) {
    dragId = id;
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", id);
    }
  }

  function onDragOver(event: DragEvent) {
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  }

  async function onDrop(event: DragEvent, targetId: string) {
    event.preventDefault();
    if (!dragId || dragId === targetId) return;
    const from = scripts.findIndex((item) => item.id === dragId);
    const to = scripts.findIndex((item) => item.id === targetId);
    if (from < 0 || to < 0) return;
    const next = [...scripts];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    scripts = next.map((item, index) => ({ ...item, sort_order: index }));
    dragId = null;
    try {
      await api.reorderScripts(scripts.map((item) => item.id));
    } catch (error) {
      showToast(friendlyError(error), "error");
      await refreshScripts();
    }
  }

  function switchTab(tab: Tab) {
    pageAction.set(null);
    activeTab = tab;
    if (tab === "runtimes" && runtimes.length === 0) loadRuntimes(false);
  }

  onMount(() => {
    loadCatalog();
    refreshScripts(true).finally(() => { loading = false; });
    adoptExternalRuns();
    api.setUiVisible(true).catch(() => {});
    api.getSettings().then((settings: Settings) => {
      if (settings.first_run_done !== "true") firstRun = true;
    }).catch(() => {});
    const onVisibility = () => { api.setUiVisible(!document.hidden).catch(() => {}); };
    document.addEventListener("visibilitychange", onVisibility);
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    (async () => {
      const unlistenState = await listen<RunStateEvent>("run-state", (event) => {
        const payload = event.payload;
        scripts = scripts.map((script) => script.id === payload.script_id
          ? { ...script, status: payload.status, last_error: payload.error, last_run_id: payload.run_id }
          : script);
        if (payload.status === "failed" && payload.error) showToast(`${scriptName(payload.script_id)}：${payload.error}`, "error");
      });
      const unlistenScripts = await listen("scripts-changed", () => { refreshScripts(); });
      const unlistenNotify = await listen<{ title: string; message: string }>("app-notification", (event) => {
        showToast(`${event.payload.title}：${event.payload.message}`);
      });
      const unlistenTrigger = await listen<{ trigger_name: string }>("trigger-fired", (event) => {
        showToast(`触发器「${event.payload.trigger_name}」已执行`);
      });
      if (disposed) {
        unlistenState();
        unlistenScripts();
        unlistenNotify();
        unlistenTrigger();
      } else {
        unlisteners.push(unlistenState, unlistenScripts, unlistenNotify, unlistenTrigger);
      }
    })();
    return () => {
      disposed = true;
      api.setUiVisible(false).catch(() => {});
      document.removeEventListener("visibilitychange", onVisibility);
      unlisteners.forEach((fn) => fn());
    };
  });

  function scriptName(id: string): string {
    return scripts.find((s) => s.id === id)?.name ?? "脚本";
  }
</script>

<svelte:head><title>LiteScriptManager</title></svelte:head>

<div class="shell">
  <aside class="sidebar">
    <div class="brand">
      <div class="brand-mark">LS</div>
      <div>
        <div class="brand-name">LiteScriptManager</div>
        <div class="brand-sub">轻量脚本管理器</div>
      </div>
    </div>

    <nav class="nav">
      <button class:active={activeTab === "scripts"} title="脚本" onclick={() => switchTab("scripts")}>
        <span class="nav-icon"><Icon name="grid" size={17} /></span> 脚本
        <span class="nav-count">{scripts.length}</span>
      </button>
      <button class:active={activeTab === "schedules"} title="计划任务" onclick={() => switchTab("schedules")}>
        <span class="nav-icon"><Icon name="clock" size={17} /></span> 计划任务
      </button>
      <button class:active={activeTab === "triggers"} title="事件触发器" onclick={() => switchTab("triggers")}>
        <span class="nav-icon"><Icon name="bolt" size={17} /></span> 事件触发器
      </button>
      <button class:active={activeTab === "history"} title="运行历史" onclick={() => switchTab("history")}>
        <span class="nav-icon"><Icon name="history" size={17} /></span> 运行历史
      </button>
      <button class:active={activeTab === "runtimes"} title="运行时环境" onclick={() => switchTab("runtimes")}>
        <span class="nav-icon"><Icon name="code" size={17} /></span> 运行时环境
      </button>
      <button class:active={activeTab === "settings"} title="设置" onclick={() => switchTab("settings")}>
        <span class="nav-icon"><Icon name="sliders" size={17} /></span> 设置
      </button>
    </nav>

    <div class="sidebar-bottom">
      <div class="mini-card">
        <div class="mini-card-top"><span class="status-dot running"></span> 后台服务</div>
        <div class="mini-card-value">{runningCount} 个脚本运行中</div>
        <div class="mini-card-hint">关闭窗口不会停止脚本</div>
      </div>
      <div class="version">v0.1.0 · 托盘常驻</div>
    </div>
  </aside>

  <main class="workspace">
    <header class="topbar">
      <div>
        <h1>{tabTitle}</h1>
        <p>{tabSubtitle}</p>
      </div>
      <div class="topbar-actions">
        {#if activeTab === "scripts"}
          <button class="btn btn-ghost" onclick={openAddPath}><Icon name="folder" size={15} /> 从路径添加</button>
          <button class="btn btn-primary" onclick={() => { showAdd = true; addMode = "new"; }}><Icon name="plus" size={15} /> 新建脚本</button>
        {:else if activeTab === "runtimes"}
          <button class="btn" disabled={runtimeLoading} onclick={() => loadRuntimes(true)}><Icon name="refresh" size={15} /> {runtimeLoading ? "检测中…" : "重新检测"}</button>
        {:else if $pageAction}
          <button
            class="btn"
            class:btn-primary={$pageAction.kind !== "danger"}
            class:btn-danger={$pageAction.kind === "danger"}
            disabled={$pageAction.disabled}
            onclick={$pageAction.run}
          >
            <Icon name={$pageAction.icon} size={15} /> {$pageAction.label}
          </button>
        {/if}
      </div>
    </header>

    {#if activeTab === "scripts"}
      <section class="content">
        <div class="toolbar">
          <div class="search-wrap">
            <span class="search-icon"><Icon name="search" size={16} /></span>
            <input class="input search" placeholder="搜索名称、路径或运行时…" bind:value={search} />
          </div>
          <div class="toolbar-meta">{filteredScripts.length} 个脚本</div>
        </div>

        {#if loading}
          <div class="empty-state"><div class="spinner"></div>正在加载脚本…</div>
        {:else if filteredScripts.length === 0}
          <div class="empty-state">
            <div class="empty-icon"><Icon name="grid" size={34} strokeWidth={1.25} /></div>
            <h2>{scripts.length === 0 ? "还没有脚本" : "没有匹配的脚本"}</h2>
            <p>{scripts.length === 0 ? "从已有文件添加，或直接创建一个新脚本。" : "换一个关键词试试。"}</p>
            {#if scripts.length === 0}
              <div class="empty-actions">
                <button class="btn btn-ghost" onclick={openAddPath}>从路径添加</button>
                <button class="btn btn-primary" onclick={() => { showAdd = true; addMode = "new"; }}><Icon name="plus" size={15} /> 新建脚本</button>
              </div>
            {/if}
          </div>
        {:else}
          <div class="script-list" role="list">
            <div class="list-head" role="presentation">
              <span class="col-drag"></span>
              <span>脚本</span>
              <span>运行时</span>
              <span>状态</span>
              <span class="col-actions">操作</span>
            </div>
            {#each filteredScripts as script, index (script.id)}
              <div
                class="script-row"
                role="listitem"
                class:dragging={dragId === script.id}
                draggable="true"
                ondragstart={(event) => onDragStart(event, script.id)}
                ondragover={onDragOver}
                ondrop={(event) => onDrop(event, script.id)}
                ondragend={() => { dragId = null; }}
              >
                <span class="drag-handle" title="拖动排序">⠿</span>
                <div class="script-main">
                  <div class="script-name-line">
                    <span class="script-name">{script.name}</span>
                    {#if !script.enabled}<span class="badge">已禁用</span>{/if}
                    {#if script.last_error}<span class="error-dot" title={script.last_error}></span>{/if}
                  </div>
                  <div class="script-path mono" title={script.path}>{displayPath(script.path)}</div>
                </div>
                <div><span class="badge runtime-badge">{runtimeName(script.runtime_id)}</span></div>
                <div class="status-cell">
                  <span class="status-dot {script.status}"></span>
                  <span>{statusLabel(script.status)}</span>
                </div>
                <div class="row-actions">
                  <button class="btn btn-sm run-btn" class:stop={script.status === "running"} disabled={!script.enabled || script.status === "starting" || script.status === "stopping" || script.status === "queued"} onclick={() => toggleScript(script)}>
                    {#if script.status === "running"}<Icon name="stop" size={12} /> 停止{:else if script.status === "starting"}启动中…{:else if script.status === "stopping"}停止中…{:else if script.status === "queued"}排队中…{:else}<Icon name="play" size={12} /> 启动{/if}
                  </button>
                  <button class="icon-btn btn btn-ghost" title="编辑" aria-label="编辑脚本" onclick={() => openEdit(script)}><Icon name="edit" size={15} /></button>
                  <button class="icon-btn btn btn-ghost" title="移除" aria-label="移除脚本" onclick={() => deleteScript(script)}><Icon name="trash" size={15} /></button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </section>
    {:else if activeTab === "schedules"}
      <section class="content"><Schedules {scripts} notify={showToast} /></section>
    {:else if activeTab === "triggers"}
      <section class="content"><Triggers {scripts} notify={showToast} /></section>
    {:else if activeTab === "history"}
      <section class="content"><History {scripts} notify={showToast} /></section>
    {:else if activeTab === "runtimes"}
      <section class="content">
        <div class="runtime-summary">
          <div><strong>{runtimes.filter((r) => r.installed).length}</strong><span> 已就绪</span></div>
          <div><strong>{runtimes.filter((r) => !r.installed).length}</strong><span> 未检测到</span></div>
          <div class="muted tiny">仅检测，不安装、不修改 PATH</div>
        </div>
        {#if runtimeLoading && runtimes.length === 0}
          <div class="empty-state"><div class="spinner"></div>正在检测运行时…</div>
        {:else if runtimes.length === 0}
          <div class="empty-state">
            <div class="empty-icon"><Icon name="code" size={34} strokeWidth={1.25} /></div>
            <h2>尚未检测</h2><p>点击右上角“重新检测”开始扫描本机环境。</p>
            <button class="btn btn-primary" onclick={() => loadRuntimes(true)}>开始检测</button>
          </div>
        {:else}
          <div class="runtime-grid">
            {#each runtimes as runtime (runtime.id)}
              <div class="runtime-card" class:missing={!runtime.installed}>
                <div class="runtime-head">
                  <span class="status-dot" class:running={runtime.installed}></span>
                  <strong>{runtime.name}</strong>
                  <span class="badge">{runtime.status === "multiple" ? `${runtime.candidates.length} 个安装` : runtime.installed ? "已就绪" : "未找到"}</span>
                </div>
                <div class="runtime-version">{runtime.version ? `v${runtime.version}` : runtime.installed ? "版本未知" : "—"}</div>
                <div class="runtime-path mono" title={runtime.exe_path ?? ""}>{runtime.exe_path ?? "未检测到可执行文件"}</div>
                {#if runtime.error}<div class="runtime-error">{runtime.error}</div>{/if}
                {#if runtime.details?.length}
                  <div class="runtime-details">{#each runtime.details as detail}<div>{detail}</div>{/each}</div>
                {/if}
                {#if runtime.candidates.length > 1}
                  <details class="runtime-candidates">
                    <summary>{runtime.candidates.length} 个候选</summary>
                    {#each runtime.candidates.slice(0, 6) as candidate}
                      <div class="candidate-row">
                        <span class="badge">{candidate.source}</span>
                        {#if candidate.kind === "gui"}<span class="badge">GUI</span>{/if}
                        <span class="mono" title={candidate.path}>{candidate.path}</span>
                      </div>
                    {/each}
                  </details>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      </section>
    {:else}
      <section class="content"><SettingsPage notify={showToast} /></section>
    {/if}
  </main>
</div>

{#if showAdd}
  <div class="modal-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) showAdd = false; }}>
    <div class="modal" role="dialog" aria-modal="true" aria-label="添加脚本">
      <div class="modal-head"><div class="modal-title">添加脚本</div><button class="btn btn-ghost icon-btn" title="关闭" aria-label="关闭" onclick={() => showAdd = false}><Icon name="close" size={17} /></button></div>
      <div class="modal-body">
        <div class="segmented">
          <button class:active={addMode === "path"} onclick={() => addMode = "path"}>从路径添加</button>
          <button class:active={addMode === "new"} onclick={() => addMode = "new"}>新建脚本</button>
        </div>
        {#if addMode === "path"}
          <div class="path-picker">
            <div class="path-value mono">{selectedPath || "尚未选择文件"}</div>
            <button class="btn" onclick={choosePath}>选择脚本文件…</button>
          </div>
          <p class="muted tiny">根据扩展名自动识别运行时；文件保持在原位置，不会复制。</p>
        {:else}
          <div class="field"><label class="label" for="new-name">名称</label><input id="new-name" class="input" placeholder="例如：每日备份" bind:value={newName} /></div>
          <div class="field"><label class="label" for="new-runtime">运行时</label>
            <select id="new-runtime" class="select" bind:value={newRuntime} onchange={() => { newContent = templateFor(newRuntime); }}>
              {#each catalog as runtime (runtime.id)}<option value={runtime.id}>{runtime.name} ({runtime.extensions.join(", ")})</option>{/each}
            </select>
          </div>
          <div class="field"><label class="label" for="new-content">初始内容</label><textarea id="new-content" class="textarea editor" bind:value={newContent}></textarea></div>
          <p class="muted tiny">文件会创建在应用脚本库目录中；后续可替换为任意路径。</p>
        {/if}
      </div>
      <div class="modal-foot">
        <button class="btn" onclick={() => showAdd = false}>取消</button>
        <button class="btn btn-primary" disabled={addMode === "path" ? !selectedPath : !newName.trim()} onclick={addMode === "path" ? addFromPath : createNewScript}>添加</button>
      </div>
    </div>
  </div>
{/if}

{#if editScript}
  <div class="modal-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) editScript = null; }}>
    <div class="modal" role="dialog" aria-modal="true" aria-label="编辑脚本">
      <div class="modal-head"><div class="modal-title">编辑脚本</div><button class="btn btn-ghost icon-btn" title="关闭" aria-label="关闭" onclick={() => editScript = null}><Icon name="close" size={17} /></button></div>
      <div class="modal-body">
        <div class="field"><label class="label" for="edit-name">名称</label><input id="edit-name" class="input" bind:value={editScript.name} /></div>
        <div class="field"><div class="label">路径</div><div class="path-value mono">{editScript.path}</div></div>
        <div class="field"><label class="label" for="edit-args">参数（每行一个）</label><textarea id="edit-args" class="textarea small" bind:value={editArgsText}></textarea></div>
        <div class="field"><label class="label" for="edit-cwd">工作目录（可选）</label><input id="edit-cwd" class="input mono" bind:value={editScript.cwd} /></div>
        <div class="field"><label class="label" for="edit-interpreter">解释器路径（可选）</label><input id="edit-interpreter" class="input mono" placeholder="留空则使用自动检测结果" bind:value={editInterpreterPath} /></div>
        <div class="field"><label class="label" for="edit-process-match">进程匹配（可选）</label><input id="edit-process-match" class="input mono" placeholder="例如：pi-web；留空则按脚本路径匹配" bind:value={editProcessMatch} /></div>
        <div class="form-grid">
          <div class="field"><label class="label" for="edit-stop-timeout">停止超时（秒）</label><input id="edit-stop-timeout" class="input" type="number" min="1" max="60" bind:value={editScript.stop_timeout_sec} /></div>
          <div class="field"><label class="label" for="edit-max-runtime">最大运行秒数（可选）</label><input id="edit-max-runtime" class="input" type="number" min="0" placeholder="留空不限制" bind:value={editScript.timeout_sec} /></div>
        </div>
        <div class="form-grid">
          <div class="field"><label class="label" for="edit-stop-mode">停止方式</label>
            <select id="edit-stop-mode" class="select" bind:value={editScript.stop_mode}>
              <option value="graceful">优雅停止，超时强杀</option>
              <option value="force">立即强制终止</option>
            </select>
          </div>
          <div class="field"><label class="label" for="edit-success">成功退出码</label><input id="edit-success" class="input mono" bind:value={editScript.success_codes} placeholder="0 或 0,2,10-12" /></div>
        </div>
        <div class="form-grid">
          <div class="field"><label class="label" for="edit-instance-policy">实例策略</label>
            <select id="edit-instance-policy" class="select" bind:value={editScript.instance_policy}>
              <option value="parallel">允许并行</option>
              <option value="skip">已有实例时跳过</option>
              <option value="queue">排队等待</option>
              <option value="replace">停止旧实例后启动</option>
            </select>
          </div>
          <div class="field"><label class="label" for="edit-max-instances">最大实例数</label><input id="edit-max-instances" class="input" type="number" min="1" max="32" bind:value={editScript.max_instances} /></div>
        </div>
        <div class="field">
          <div class="label">内容哈希</div>
          <div class="hash-row">
            <span class="mono tiny">{editScript.content_hash ?? "未固定"}</span>
            <button class="btn btn-sm" onclick={toggleHashPin}>{editScript.content_hash ? "取消固定" : "固定当前内容"}</button>
          </div>
          <p class="muted tiny">固定后，设置中的“运行前校验”开启时会在每次启动前检查脚本是否被替换。</p>
        </div>
        <label class="check-row"><input type="checkbox" bind:checked={editScript.enabled} /> 启用此脚本（显示在快捷面板中）</label>
      </div>
      <div class="modal-foot"><button class="btn" onclick={() => editScript = null}>取消</button><button class="btn btn-primary" onclick={saveEdit}>保存</button></div>
    </div>
  </div>
{/if}

{#if firstRun}
  <div class="modal-backdrop" role="presentation">
    <div class="modal" role="dialog" aria-modal="true" aria-label="首次启动">
      <div class="modal-head"><div class="modal-title">欢迎使用 LiteScriptManager</div></div>
      <div class="modal-body">
        <p class="first-run-lead">托盘常驻、按需创建 WebView。关闭窗口不会停止脚本，退出应用才会停止全部受管进程。</p>
        <div class="first-run-list">
          <div><strong>1. 添加脚本</strong><span>支持 14 种运行时，自动检测解释器，不安装、不修改 PATH。</span></div>
          <div><strong>2. 设置计划与触发器</strong><span>定时、循环重启、进程/资源事件都可以启动或停止脚本。</span></div>
          <div><strong>3. 查看历史与日志</strong><span>每次运行都有独立日志文件，历史和日志按保留策略清理。</span></div>
        </div>
        <p class="muted tiny">安全提示：脚本以当前用户权限运行，不默认提升管理员权限。可在设置中限制脚本根目录并固定内容哈希。</p>
      </div>
      <div class="modal-foot">
        <button class="btn btn-primary" disabled={firstRunSaving} onclick={dismissFirstRun}>{firstRunSaving ? "保存中…" : "开始使用"}</button>
      </div>
    </div>
  </div>
{/if}

{#if toast}
  <div class="toast" class:error={toast.kind === "error"}>{toast.message}</div>
{/if}

<style>
  .shell {
    display: grid; grid-template-columns: 232px minmax(0, 1fr); gap: 12px;
    height: 100vh; padding: 12px; background: var(--bg);
  }
  .sidebar {
    display: flex; flex-direction: column; padding: 20px 13px 14px;
    background: #f8f8fa; border-radius: 18px; min-width: 0;
  }
  .brand { display: flex; gap: 10px; align-items: center; padding: 4px 8px 28px; }
  .brand-mark {
    width: 31px; height: 31px; border-radius: 9px; display: grid; place-items: center;
    background: #e8eefb; color: #4a6bb8; font-weight: 700; font-size: 10px; letter-spacing: -.2px;
  }
  .brand-name { font-size: 13px; font-weight: 600; letter-spacing: -.1px; color: #343941; }
  .brand-sub { color: #a4aab4; font-size: 9.5px; margin-top: 3px; letter-spacing: .1px; }
  .nav { display: grid; gap: 3px; }
  .nav button {
    display: flex; align-items: center; gap: 10px; height: 40px; padding: 0 12px;
    border: 0; border-radius: 11px; background: transparent; color: #707784;
    font-size: 13px; text-align: left; transition: background .15s, color .15s;
  }
  .nav button:hover { background: #f0f1f5; color: #414852; }
  .nav button.active { background: #e9effb; color: #37578f; font-weight: 550; }
  .nav-icon { width: 18px; display: grid; place-items: center; color: #9aa1ac; flex: 0 0 auto; }
  .nav button.active .nav-icon { color: #5a79c4; }
  .nav-count { margin-left: auto; font-size: 10px; color: #a1a7b1; padding: 1px 2px; }
  .nav button.active .nav-count { color: #6d84b8; }
  .sidebar-bottom { margin-top: auto; }
  .mini-card {
    border: 1px solid #eef0f4; background: rgba(255, 255, 255, .78);
    border-radius: 14px; padding: 14px; box-shadow: 0 1px 2px rgba(35, 44, 62, .025);
  }
  .mini-card-top { display: flex; gap: 8px; align-items: center; color: #858c97; font-size: 10.5px; }
  .mini-card-value { font-size: 12px; margin-top: 10px; color: #444a53; font-weight: 550; }
  .mini-card-hint { font-size: 9.5px; color: #a7adb7; margin-top: 4px; line-height: 1.45; }
  .version { color: #b3b8c1; font-size: 9.5px; padding: 14px 4px 2px; }
  .workspace {
    min-width: 0; min-height: 0; display: flex; flex-direction: column; overflow: hidden;
    background: #fff; border: 1px solid #eef0f4; border-radius: 18px;
  }
  .topbar {
    flex: 0 0 auto;
    display: flex; align-items: flex-start; justify-content: space-between; gap: 20px;
    padding: 30px 34px 18px;
  }
  .topbar h1 { margin: 0; font-size: 20px; letter-spacing: -.3px; font-weight: 600; color: #30353d; }
  .topbar p { margin: 7px 0 0; color: #969da8; font-size: 11.5px; line-height: 1.5; }
  .topbar-actions { display: flex; gap: 7px; padding-top: 2px; }
  .content { flex: 1; min-height: 0; overflow: auto; overscroll-behavior: contain; padding: 8px 34px 34px; }
  .toolbar { display: flex; align-items: center; gap: 14px; margin-bottom: 15px; }
  .search-wrap { position: relative; width: min(360px, 100%); }
  .search-icon {
    position: absolute; left: 12px; top: 50%; transform: translateY(-50%);
    color: #a8aeb8; line-height: 1; pointer-events: none; display: flex;
  }
  .search { padding-left: 36px; height: 36px; }
  .toolbar-meta { margin-left: auto; color: #a8aeb8; font-size: 10.5px; }
  .script-list { border: 1px solid #eef0f4; border-radius: 16px; overflow: hidden; background: #fff; }
  .list-head, .script-row {
    display: grid; grid-template-columns: 20px minmax(180px, 1fr) 116px 76px 130px;
    gap: 12px; align-items: center;
  }
  .list-head {
    padding: 12px 16px; color: #979eaa; font-size: 9.5px; text-transform: uppercase;
    letter-spacing: .08em; background: #fbfbfc; border-bottom: 1px solid #f1f2f5;
  }
  .col-actions { text-align: right; }
  .script-row { padding: 14px 16px; border-bottom: 1px solid #f3f4f6; transition: background .12s, opacity .12s; }
  .script-row:last-child { border-bottom: 0; }
  .script-row:hover { background: #fbfcfe; }
  .script-row.dragging { opacity: .42; }
  .drag-handle { color: #c3c8d1; font-size: 14px; cursor: grab; user-select: none; }
  .script-main { min-width: 0; }
  .script-name-line { display: flex; align-items: center; gap: 7px; min-width: 0; }
  .script-name { font-size: 12.8px; font-weight: 550; color: #383e47; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .script-path { color: #a3a9b3; font-size: 10px; margin-top: 5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .error-dot { width: 5px; height: 5px; background: var(--danger); border-radius: 50%; flex: 0 0 auto; }
  .runtime-badge { color: #7b8494; background: #f4f6fa; }
  .status-cell { display: flex; align-items: center; gap: 7px; color: #7f8793; font-size: 10.5px; white-space: nowrap; }
  .row-actions { display: flex; justify-content: flex-end; gap: 3px; }
  .run-btn { min-width: 64px; background: #eef4fc; color: #4c6da9; }
  .run-btn:hover { background: #e3edfb; color: #3d5f9f; }
  .run-btn.stop { color: #b46060; background: #fbf0f0; }
  .run-btn.stop:hover { background: #f9e7e7; }
  .empty-state { min-height: 300px; }
  .empty-actions { display: flex; gap: 8px; }
  .runtime-summary {
    display: flex; align-items: baseline; gap: 28px; padding: 16px 18px;
    border: 1px solid #eef0f4; border-radius: 16px; background: #fbfbfc; margin-bottom: 14px;
  }
  .runtime-summary strong { font-size: 20px; color: #39404a; margin-right: 3px; font-weight: 550; }
  .runtime-summary span { color: #8b929d; font-size: 10.5px; }
  .runtime-summary .tiny { margin-left: auto; color: #a8aeb8; }
  .runtime-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(245px, 1fr)); gap: 10px; }
  .runtime-card { padding: 16px; border: 1px solid #eef0f4; border-radius: 16px; background: #fff; min-width: 0; transition: border-color .15s, background .15s; }
  .runtime-card:hover { border-color: #e4e8f0; background: #fdfdff; }
  .runtime-card.missing { opacity: .66; }
  .runtime-head { display: flex; align-items: center; gap: 8px; }
  .runtime-head strong { font-size: 12.5px; font-weight: 550; color: #3d434c; }
  .runtime-head .badge { margin-left: auto; }
  .runtime-version { color: #5c7cc1; font-size: 10.5px; margin: 12px 0 5px; }
  .runtime-path { color: #a1a7b2; font-size: 9.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .runtime-error { color: #b36666; font-size: 10px; margin-top: 7px; line-height: 1.45; }
  .path-picker { display: flex; gap: 10px; align-items: center; margin: 18px 0 10px; }
  .path-value {
    flex: 1; min-width: 0; border: 1px solid #f0f1f4; background: #f7f8fa; color: #8b929e;
    border-radius: 10px; padding: 9px 11px; font-size: 10.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .small { min-height: 74px; font-size: 11.5px; }
  .editor { font-family: "Cascadia Code", Consolas, monospace; font-size: 11.5px; }
  .form-grid { gap: 11px; }
  .hash-row {
    display: flex; align-items: center; gap: 10px; justify-content: space-between;
    border: 1px solid #f0f1f4; background: #f7f8fa; border-radius: 10px; padding: 8px 11px;
  }
  .hash-row .mono { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: #9aa1ac; }
  .runtime-details { margin-top: 10px; border-top: 1px solid #f2f3f6; padding-top: 9px; color: #9aa1ac; font-size: 9.5px; line-height: 1.55; }
  .runtime-candidates { margin-top: 8px; font-size: 9.5px; color: #9aa1ac; }
  .runtime-candidates summary { cursor: pointer; color: #aeb4be; }
  .candidate-row { display: flex; align-items: center; gap: 5px; margin-top: 6px; min-width: 0; }
  .candidate-row .mono { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .first-run-lead { color: #525964; font-size: 12px; line-height: 1.7; margin: 0 0 17px; }
  .first-run-list { display: grid; gap: 9px; margin-bottom: 15px; }
  .first-run-list div { border: 1px solid #f0f2f5; background: #fbfbfc; border-radius: 12px; padding: 12px 13px; }
  .first-run-list strong { display: block; font-size: 11.5px; margin-bottom: 3px; color: #414852; }
  .first-run-list span { color: #9299a4; font-size: 10.5px; line-height: 1.5; }
  @media (max-width: 980px) {
    .shell { grid-template-columns: 72px minmax(0, 1fr); }
    .brand { justify-content: center; padding-left: 0; padding-right: 0; }
    .brand > div:last-child, .nav-count { display: none; }
    .nav button { justify-content: center; padding: 0; font-size: 0; }
    .nav-icon { width: auto; }
    .sidebar-bottom { display: none; }
    .topbar { padding: 24px 24px 16px; }
    .content { padding: 6px 24px 26px; }
  }
  @media (max-width: 900px) {
    .list-head, .script-row { grid-template-columns: 20px minmax(150px, 1fr) 90px 130px; }
    .list-head span:nth-child(3), .script-row > div:nth-child(3) { display: none; }
  }
</style>
