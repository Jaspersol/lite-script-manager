<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, friendlyError } from "./lib/api";
  import type { RunStateEvent, RuntimeInfo, RuntimeMeta, Script } from "./lib/types";

  type Tab = "scripts" | "runtimes" | "settings";

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
  let dragId: string | null = $state(null);

  const scriptExtensions = ["py", "js", "mjs", "cjs", "sh", "bash", "pl", "php", "R", "r", "lua", "bat", "cmd", "ps1", "vbs", "vbe", "wsf", "ahk", "au3"];

  const filteredScripts = $derived(
    scripts.filter((script) => {
      const term = search.trim().toLowerCase();
      if (!term) return true;
      return `${script.name} ${script.path} ${script.runtime_id}`.toLowerCase().includes(term);
    }),
  );

  const runningCount = $derived(scripts.filter((s) => s.status === "running" || s.status === "starting").length);

  function runtimeName(id: string): string {
    return catalog.find((item) => item.id === id)?.name ?? id;
  }

  function isBusy(script: Script): boolean {
    return script.status === "starting" || script.status === "stopping";
  }

  function statusLabel(status: string): string {
    return {
      stopped: "已停止",
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
  }

  async function saveEdit() {
    if (!editScript) return;
    try {
      await api.updateScript(
        editScript.id,
        editScript.name,
        editArgsText.split("\n").map((arg) => arg.trim()).filter((arg) => arg.length > 0),
        editScript.cwd || null,
        editInterpreterPath.trim() || null,
        editScript.stop_timeout_sec || 3,
        editScript.enabled,
      );
      editScript = null;
      await refreshScripts();
      showToast("脚本配置已保存");
    } catch (error) {
      showToast(friendlyError(error), "error");
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
      if (disposed) {
        unlistenState();
        unlistenScripts();
      } else {
        unlisteners.push(unlistenState, unlistenScripts);
      }
    })();
    return () => {
      disposed = true;
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
      <div class="version">P0 · v0.1.0</div>
    </div>
  </aside>

  <main class="workspace">
    <header class="topbar">
      <div>
        <h1>{activeTab === "scripts" ? "脚本" : activeTab === "runtimes" ? "运行时环境" : "设置"}</h1>
        <p>{activeTab === "scripts" ? "管理本机脚本，按顺序显示在托盘快捷面板中。" : activeTab === "runtimes" ? "只检测，不安装；检测结果用于运行前预检。" : "应用与窗口行为设置。"}</p>
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
                  <button class="btn btn-sm run-btn" class:stop={script.status === "running"} disabled={!script.enabled || script.status === "starting" || script.status === "stopping"} onclick={() => toggleScript(script)}>
                    {#if script.status === "running"}■ 停止{:else if script.status === "starting"}启动中…{:else if script.status === "stopping"}停止中…{:else}▶ 启动{/if}
                  </button>
                  <button class="btn btn-sm btn-ghost" onclick={() => openEdit(script)}>编辑</button>
                  <button class="icon-btn btn btn-ghost" title="移除" onclick={() => deleteScript(script)}>×</button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </section>
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
              </div>
            {/each}
          </div>
        {/if}
      </section>
    {:else}
      <section class="content">
        <div class="settings-card">
          <h2>应用设置</h2>
          <p class="muted">P0 先提供基础能力。计划任务、事件触发器、快捷键和日志页将在后续阶段加入。</p>
          <div class="setting-row"><div><strong>关闭窗口</strong><span>关闭主窗口时销毁 WebView，脚本继续在后台运行。</span></div><span class="badge">已启用</span></div>
          <div class="setting-row"><div><strong>托盘面板</strong><span>左键打开快捷面板；右键打开 Show LSM / Exit LSM。</span></div><span class="badge">已启用</span></div>
          <div class="setting-row"><div><strong>数据目录</strong><span>脚本配置、运行记录与日志保存在应用数据目录。</span></div><span class="badge">SQLite</span></div>
        </div>
      </section>
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
        <div class="field"><label class="label" for="edit-stop-timeout">停止超时（秒）</label><input id="edit-stop-timeout" class="input" type="number" min="1" max="60" bind:value={editScript.stop_timeout_sec} /></div>
        <label class="check-row"><input type="checkbox" bind:checked={editScript.enabled} /> 启用此脚本（显示在快捷面板中）</label>
      </div>
      <div class="modal-foot"><button class="btn" onclick={() => editScript = null}>取消</button><button class="btn btn-primary" onclick={saveEdit}>保存</button></div>
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
  .settings-card h2 { font-size: 15px; margin: 0 0 6px; }
  .settings-card p { font-size: 12px; line-height: 1.6; margin: 0 0 18px; }
  .setting-row { display: flex; justify-content: space-between; gap: 20px; border-top: 1px solid var(--line-soft); padding: 15px 0; }
  .setting-row strong { display: block; font-size: 12.5px; margin-bottom: 4px; }
  .setting-row span { color: var(--muted); font-size: 11px; line-height: 1.45; }
  .path-picker { display: flex; gap: 10px; align-items: center; margin: 18px 0 10px; }
  .path-value { flex: 1; min-width: 0; border: 1px solid var(--line); background: #0f141d; color: var(--muted); border-radius: 8px; padding: 9px 10px; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .small { min-height: 74px; font-size: 12px; }
  .editor { font-family: "Cascadia Code", Consolas, monospace; font-size: 12px; }
  .check-row { display: flex; align-items: center; gap: 8px; color: var(--muted); font-size: 12px; }
  .check-row input { accent-color: var(--accent-strong); }
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
