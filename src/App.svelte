<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, friendlyError } from "./lib/api";
  import Schedules from "./lib/Schedules.svelte";
  import Triggers from "./lib/Triggers.svelte";
  import History from "./lib/History.svelte";
  import SettingsPage from "./lib/SettingsPage.svelte";
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
      <button class:active={activeTab === "scripts"} onclick={() => switchTab("scripts")}>
        <span class="nav-icon">▤</span> 脚本
        <span class="nav-count">{scripts.length}</span>
      </button>
      <button class:active={activeTab === "schedules"} onclick={() => switchTab("schedules")}>
        <span class="nav-icon">◷</span> 计划任务
      </button>
      <button class:active={activeTab === "triggers"} onclick={() => switchTab("triggers")}>
        <span class="nav-icon">⚡</span> 事件触发器
      </button>
      <button class:active={activeTab === "history"} onclick={() => switchTab("history")}>
        <span class="nav-icon">≡</span> 运行历史
      </button>
      <button class:active={activeTab === "runtimes"} onclick={() => switchTab("runtimes")}>
        <span class="nav-icon">◇</span> 运行时环境
      </button>
      <button class:active={activeTab === "settings"} onclick={() => switchTab("settings")}>
        <span class="nav-icon">⚙</span> 设置
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
          <button class="btn" onclick={openAddPath}>从路径添加</button>
          <button class="btn btn-primary" onclick={() => { showAdd = true; addMode = "new"; }}>+ 新建脚本</button>
        {:else if activeTab === "runtimes"}
          <button class="btn" disabled={runtimeLoading} onclick={() => loadRuntimes(true)}>{runtimeLoading ? "检测中…" : "重新检测"}</button>
        {/if}
      </div>
    </header>

    {#if activeTab === "scripts"}
      <section class="content">
        <div class="toolbar">
          <div class="search-wrap">
            <span class="search-icon">⌕</span>
            <input class="input search" placeholder="搜索名称、路径或运行时…" bind:value={search} />
          </div>
          <div class="toolbar-meta">{filteredScripts.length} 个脚本</div>
        </div>

        {#if loading}
          <div class="empty-state"><div class="spinner"></div>正在加载脚本…</div>
        {:else if filteredScripts.length === 0}
          <div class="empty-state">
            <div class="empty-icon">▤</div>
            <h2>{scripts.length === 0 ? "还没有脚本" : "没有匹配的脚本"}</h2>
            <p>{scripts.length === 0 ? "从已有文件添加，或直接创建一个新脚本。" : "换一个关键词试试。"}</p>
            {#if scripts.length === 0}
              <div class="empty-actions">
                <button class="btn" onclick={openAddPath}>从路径添加</button>
                <button class="btn btn-primary" onclick={() => { showAdd = true; addMode = "new"; }}>+ 新建脚本</button>
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
                  <div class="script-path mono" title={script.path}>{script.path}</div>
                </div>
                <div><span class="badge runtime-badge">{runtimeName(script.runtime_id)}</span></div>
                <div class="status-cell">
                  <span class="status-dot {script.status}"></span>
                  <span>{statusLabel(script.status)}</span>
                </div>
                <div class="row-actions">
                  <button class="btn btn-sm run-btn" class:stop={script.status === "running"} disabled={!script.enabled || script.status === "starting" || script.status === "stopping" || script.status === "queued"} onclick={() => toggleScript(script)}>
                    {#if script.status === "running"}■ 停止{:else if script.status === "starting"}启动中…{:else if script.status === "stopping"}停止中…{:else if script.status === "queued"}排队中…{:else}▶ 启动{/if}
                  </button>
                  <button class="btn btn-sm btn-ghost" onclick={() => openEdit(script)}>编辑</button>
                  <button class="icon-btn btn btn-ghost" title="移除" onclick={() => deleteScript(script)}>×</button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </section>
    {:else if activeTab === "schedules"}
      <Schedules {scripts} notify={showToast} />
    {:else if activeTab === "triggers"}
      <Triggers {scripts} notify={showToast} />
    {:else if activeTab === "history"}
      <History {scripts} notify={showToast} />
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
          <div class="empty-state"><h2>尚未检测</h2><p>点击右上角“重新检测”开始扫描本机环境。</p><button class="btn btn-primary" onclick={() => loadRuntimes(true)}>开始检测</button></div>
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
      <SettingsPage notify={showToast} />
    {/if}
  </main>
</div>

{#if showAdd}
  <div class="modal-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) showAdd = false; }}>
    <div class="modal" role="dialog" aria-modal="true" aria-label="添加脚本">
      <div class="modal-head"><div class="modal-title">添加脚本</div><button class="btn btn-ghost icon-btn" onclick={() => showAdd = false}>×</button></div>
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
      <div class="modal-head"><div class="modal-title">编辑脚本</div><button class="btn btn-ghost icon-btn" onclick={() => editScript = null}>×</button></div>
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
  .shell { display: grid; grid-template-columns: 224px minmax(0, 1fr); height: 100vh; }
  .sidebar { display: flex; flex-direction: column; padding: 20px 14px 16px; background: #10151e; border-right: 1px solid var(--line-soft); min-width: 0; }
  .brand { display: flex; gap: 11px; align-items: center; padding: 2px 8px 24px; }
  .brand-mark { width: 34px; height: 34px; border-radius: 10px; display: grid; place-items: center; background: linear-gradient(145deg, #5eead4, #2dd4bf); color: #06241f; font-weight: 800; font-size: 12px; letter-spacing: -.4px; }
  .brand-name { font-size: 13px; font-weight: 700; letter-spacing: .1px; }
  .brand-sub { color: var(--muted-2); font-size: 10px; margin-top: 2px; }
  .nav { display: grid; gap: 4px; }
  .nav button { display: flex; align-items: center; gap: 9px; height: 38px; padding: 0 10px; border: 0; border-radius: 8px; background: transparent; color: var(--muted); font-size: 13px; text-align: left; }
  .nav button:hover { background: #171e29; color: var(--text); }
  .nav button.active { background: #1b2432; color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
  .nav-icon { width: 16px; text-align: center; color: var(--muted-2); font-size: 13px; }
  .nav button.active .nav-icon { color: var(--accent); }
  .nav-count { margin-left: auto; font-size: 10px; color: var(--muted-2); background: #0d121a; border: 1px solid var(--line-soft); border-radius: 5px; padding: 1px 5px; }
  .sidebar-bottom { margin-top: auto; }
  .mini-card { border: 1px solid var(--line-soft); background: #121a24; border-radius: 10px; padding: 12px; }
  .mini-card-top { display: flex; gap: 7px; align-items: center; color: var(--muted); font-size: 11px; }
  .mini-card-value { font-size: 12px; margin-top: 10px; color: var(--text); }
  .mini-card-hint { font-size: 10px; color: var(--muted-2); margin-top: 4px; line-height: 1.4; }
  .version { color: #4e5a6e; font-size: 10px; padding: 14px 4px 0; }
  .workspace { min-width: 0; display: flex; flex-direction: column; overflow: hidden; }
  .topbar { display: flex; align-items: flex-start; justify-content: space-between; gap: 20px; padding: 26px 32px 20px; border-bottom: 1px solid var(--line-soft); }
  .topbar h1 { margin: 0; font-size: 22px; letter-spacing: -.4px; font-weight: 680; }
  .topbar p { margin: 5px 0 0; color: var(--muted); font-size: 12px; }
  .topbar-actions { display: flex; gap: 8px; padding-top: 2px; }
  .content { flex: 1; overflow: auto; padding: 22px 32px 32px; }
  .toolbar { display: flex; align-items: center; gap: 14px; margin-bottom: 14px; }
  .search-wrap { position: relative; width: min(380px, 100%); }
  .search-icon { position: absolute; left: 10px; top: 7px; color: var(--muted-2); font-size: 17px; line-height: 1; }
  .search { padding-left: 31px; height: 34px; }
  .toolbar-meta { margin-left: auto; color: var(--muted-2); font-size: 11px; }
  .script-list { border: 1px solid var(--line-soft); border-radius: 11px; overflow: hidden; background: #111720; }
  .list-head, .script-row { display: grid; grid-template-columns: 22px minmax(220px, 1fr) 128px 90px 210px; gap: 12px; align-items: center; }
  .list-head { padding: 10px 14px; color: var(--muted-2); font-size: 10px; text-transform: uppercase; letter-spacing: .08em; background: #0e141d; border-bottom: 1px solid var(--line-soft); }
  .col-actions { text-align: right; }
  .script-row { padding: 12px 14px; border-bottom: 1px solid #1b222c; transition: background .12s, opacity .12s; }
  .script-row:last-child { border-bottom: 0; }
  .script-row:hover { background: #151d28; }
  .script-row.dragging { opacity: .45; }
  .drag-handle { color: #4d5a6e; font-size: 15px; cursor: grab; user-select: none; }
  .script-main { min-width: 0; }
  .script-name-line { display: flex; align-items: center; gap: 7px; min-width: 0; }
  .script-name { font-size: 13px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .script-path { color: #647188; font-size: 10.5px; margin-top: 5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .error-dot { width: 6px; height: 6px; background: var(--danger); border-radius: 50%; flex: 0 0 auto; }
  .runtime-badge { color: #a8b4c8; }
  .status-cell { display: flex; align-items: center; gap: 7px; color: var(--muted); font-size: 11px; white-space: nowrap; }
  .row-actions { display: flex; justify-content: flex-end; gap: 3px; }
  .run-btn { min-width: 58px; border-color: #2d3a4d; }
  .run-btn.stop { color: #fecaca; border-color: #4a2529; background: #2a171a; }
  .empty-state { min-height: 300px; display: flex; flex-direction: column; align-items: center; justify-content: center; color: var(--muted); text-align: center; border: 1px dashed #263040; border-radius: 12px; background: rgba(17, 23, 32, .45); }
  .empty-state h2 { color: var(--text); margin: 12px 0 4px; font-size: 16px; }
  .empty-state p { margin: 0 0 16px; font-size: 12px; }
  .empty-icon { font-size: 34px; color: #3c4b60; }
  .empty-actions { display: flex; gap: 8px; }
  .spinner { width: 22px; height: 22px; border: 2px solid #2b394b; border-top-color: var(--accent); border-radius: 50%; animation: spin .8s linear infinite; margin-bottom: 12px; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .runtime-summary { display: flex; align-items: baseline; gap: 26px; padding: 14px 16px; border: 1px solid var(--line-soft); border-radius: 10px; background: #111720; margin-bottom: 16px; }
  .runtime-summary strong { font-size: 20px; color: var(--text); margin-right: 3px; }
  .runtime-summary span { color: var(--muted); font-size: 11px; }
  .runtime-summary .tiny { margin-left: auto; }
  .runtime-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(245px, 1fr)); gap: 10px; }
  .runtime-card { padding: 14px; border: 1px solid var(--line-soft); border-radius: 10px; background: #121923; min-width: 0; }
  .runtime-card.missing { opacity: .72; }
  .runtime-head { display: flex; align-items: center; gap: 8px; }
  .runtime-head strong { font-size: 12.5px; }
  .runtime-head .badge { margin-left: auto; }
  .runtime-version { color: var(--accent); font-size: 11px; margin: 12px 0 5px; }
  .runtime-path { color: #637086; font-size: 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .runtime-error { color: #fca5a5; font-size: 10px; margin-top: 7px; line-height: 1.4; }
  .settings-card { max-width: 720px; padding: 22px; border: 1px solid var(--line-soft); border-radius: 12px; background: #111720; }
    .setting-row { display: flex; justify-content: space-between; gap: 20px; border-top: 1px solid var(--line-soft); padding: 15px 0; }
  .path-picker { display: flex; gap: 10px; align-items: center; margin: 18px 0 10px; }
  .path-value { flex: 1; min-width: 0; border: 1px solid var(--line); background: #0f141d; color: var(--muted); border-radius: 8px; padding: 9px 10px; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .small { min-height: 74px; font-size: 12px; }
  .editor { font-family: "Cascadia Code", Consolas, monospace; font-size: 12px; }
  .check-row { display: flex; align-items: center; gap: 8px; color: var(--muted); font-size: 12px; }
  .check-row input { accent-color: var(--accent-strong); }
  .form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  .hash-row { display: flex; align-items: center; gap: 10px; justify-content: space-between; border: 1px solid var(--line); background: #0f141d; border-radius: 8px; padding: 8px 10px; }
  .hash-row .mono { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted); }
  .runtime-details { margin-top: 9px; border-top: 1px solid var(--line-soft); padding-top: 8px; color: var(--muted); font-size: 9.5px; line-height: 1.5; }
  .runtime-candidates { margin-top: 8px; font-size: 10px; color: var(--muted); }
  .runtime-candidates summary { cursor: pointer; color: var(--muted-2); }
  .candidate-row { display: flex; align-items: center; gap: 5px; margin-top: 6px; min-width: 0; }
  .candidate-row .mono { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .first-run-lead { color: var(--text); font-size: 12.5px; line-height: 1.6; margin: 0 0 16px; }
  .first-run-list { display: grid; gap: 10px; margin-bottom: 14px; }
  .first-run-list div { border: 1px solid var(--line-soft); background: #0f141d; border-radius: 8px; padding: 10px 11px; }
  .first-run-list strong { display: block; font-size: 12px; margin-bottom: 3px; }
  .first-run-list span { color: var(--muted); font-size: 11px; line-height: 1.45; }
  @media (max-width: 900px) {
    .shell { grid-template-columns: 68px minmax(0, 1fr); }
    .brand > div:last-child, .nav button:not(.active) .nav-count, .nav button { font-size: 0; }
    .nav button { justify-content: center; padding: 0; }
    .nav-icon { font-size: 15px; }
    .sidebar-bottom { display: none; }
    .list-head, .script-row { grid-template-columns: 20px minmax(160px, 1fr) 90px 76px; }
    .list-head span:nth-child(3), .script-row > div:nth-child(3) { display: none; }
    .row-actions .btn-ghost { display: none; }
    .topbar { padding: 20px; }
    .content { padding: 16px 20px 24px; }
  }
</style>
