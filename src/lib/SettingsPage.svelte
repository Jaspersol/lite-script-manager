<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { api, formatBytes, friendlyError } from "./api";
  import { createListenerGroup } from "./listeners";
  import { pageAction } from "./uiActions";
  import type { PageAction } from "./uiActions";
  import Icon from "./Icon.svelte";
  import type { Settings, UpdateInfo, UpdateProgress } from "./types";

  let {
    notify = () => {},
  }: {
    notify: (message: string, kind?: "info" | "error") => void;
  } = $props();

  let values: Settings = $state({});
  let loading = $state(true);
  let saving = $state(false);

  let appVersion = $state("");
  let updateInfo: UpdateInfo | null = $state(null);
  let updateError = $state("");
  let installError = $state("");
  let checking = $state(false);
  let installing = $state(false);
  let progress: UpdateProgress | null = $state(null);

  const percent = $derived.by(() => {
    if (!progress || !progress.total) return null;
    return Math.min(100, Math.round((progress.downloaded / progress.total) * 100));
  });

  async function load() {
    try {
      values = await api.getSettings();
    } catch (error) {
      notify(friendlyError(error), "error");
    } finally {
      loading = false;
    }
  }

  function bool(key: string): boolean {
    return ["1", "true", "yes", "on"].includes(values[key]);
  }

  function setBool(key: string, checked: boolean) {
    values = { ...values, [key]: checked ? "true" : "false" };
  }

  function setValue(key: string, value: string) {
    values = { ...values, [key]: value };
  }

  async function checkUpdate() {
    checking = true;
    updateError = "";
    installError = "";
    try {
      const info = await api.checkUpdate();
      updateInfo = info;
      if (!info.has_update) notify("已是最新版本");
    } catch (error) {
      updateInfo = null;
      updateError = friendlyError(error);
    } finally {
      checking = false;
    }
  }

  async function installUpdate() {
    if (!updateInfo?.download_url || !updateInfo.asset_name) return;
    installing = true;
    installError = "";
    progress = { downloaded: 0, total: updateInfo.asset_size };
    try {
      await api.installUpdate();
    } catch (error) {
      installing = false;
      progress = null;
      installError = friendlyError(error);
      notify(installError, "error");
    }
  }

  async function openReleasePage() {
    if (!updateInfo?.release_url) return;
    try {
      await api.openReleasePage(updateInfo.release_url);
    } catch (error) {
      notify(friendlyError(error), "error");
    }
  }

  function formatDate(value: string | null | undefined): string {
    if (!value) return "";
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? "" : date.toLocaleDateString("zh-CN");
  }

  async function save() {
    saving = true;
    try {
      values = await api.updateSettings(values);
      notify("设置已保存");
    } catch (error) {
      notify(friendlyError(error), "error");
    } finally {
      saving = false;
    }
  }

  $effect(() => {
    const action: PageAction = {
      label: saving ? "保存中…" : "保存设置",
      icon: "sliders",
      kind: "primary",
      disabled: saving || loading,
      run: save,
    };
    pageAction.set(action);
    return () => pageAction.set(null);
  });

  onMount(() => {
    load();
    getVersion().then((value) => { appVersion = value; }).catch(() => {});
    // Show the result of the startup check even though its event fired before this window existed.
    api.lastUpdateResult().then((info) => {
      if (info && !updateInfo) updateInfo = info;
    }).catch(() => {});
    const listeners = createListenerGroup();
    listeners.add("settings-changed", load);
    listeners.add<UpdateProgress>("update-progress", (event) => { progress = event.payload; });
    listeners.add<UpdateInfo>("update-check-result", (event) => {
      updateInfo = event.payload;
      updateError = "";
      installError = "";
    });
    return () => listeners.dispose();
  });
</script>

<div>
  {#if loading}
    <div class="empty-state"><div class="spinner"></div>正在加载设置…</div>
  {:else}
    <div class="settings-grid">
      <section class="settings-card">
        <h3>启动与窗口</h3>
        <label class="setting-check"><input type="checkbox" checked={bool("start_with_windows")} onchange={(event) => setBool("start_with_windows", event.currentTarget.checked)} /> 开机启动 LiteScriptManager</label>
        <label class="setting-check"><input type="checkbox" checked={bool("open_panel_on_start")} onchange={(event) => setBool("open_panel_on_start", event.currentTarget.checked)} /> 启动时打开快捷面板</label>
        <label class="setting-check"><input type="checkbox" checked={bool("close_main_exits")} onchange={(event) => setBool("close_main_exits", event.currentTarget.checked)} /> 关闭主窗口时退出应用并停止全部脚本</label>
        <div class="field">
          <label class="label" for="hotkey">打开面板快捷键</label>
          <input id="hotkey" class="input mono" value={values.hotkey_show_panel ?? ""} oninput={(event) => setValue("hotkey_show_panel", event.currentTarget.value)} placeholder="例如 Ctrl+Alt+L；留空禁用" />
          <p class="muted tiny">使用 Tauri 全局快捷键格式，例如 Ctrl+Shift+L 或 Alt+Space。</p>
        </div>
      </section>

      <section class="settings-card">
        <h3>运行与日志</h3>
        <div class="field">
          <label class="label" for="default-stop">默认停止超时（秒）</label>
          <input id="default-stop" class="input" type="number" min="1" max="60" value={values.default_stop_timeout ?? "3"} oninput={(event) => setValue("default_stop_timeout", event.currentTarget.value)} />
        </div>
        <div class="field">
          <label class="label" for="retention">日志保留天数</label>
          <input id="retention" class="input" type="number" min="0" value={values.log_retention_days ?? "14"} oninput={(event) => setValue("log_retention_days", event.currentTarget.value)} />
        </div>
        <div class="field">
          <label class="label" for="max-log">日志总大小上限（MB）</label>
          <input id="max-log" class="input" type="number" min="0" value={values.log_max_total_mb ?? "200"} oninput={(event) => setValue("log_max_total_mb", event.currentTarget.value)} />
        </div>
        <div class="field">
          <label class="label" for="monitor-interval">监控采样间隔（毫秒）</label>
          <input id="monitor-interval" class="input" type="number" min="250" max="60000" value={values.monitor_interval_ms ?? "1000"} oninput={(event) => setValue("monitor_interval_ms", event.currentTarget.value)} />
        </div>
      </section>

      <section class="settings-card">
        <h3>安全</h3>
        <label class="setting-check"><input type="checkbox" checked={bool("verify_script_hash")} onchange={(event) => setBool("verify_script_hash", event.currentTarget.checked)} /> 运行前校验已固定的脚本内容哈希</label>
        <div class="field">
          <label class="label" for="allowed-roots">允许的脚本根目录（每行一个）</label>
          <textarea id="allowed-roots" class="textarea mono" value={values.allowed_roots ?? ""} oninput={(event) => setValue("allowed_roots", event.currentTarget.value)} placeholder="留空表示不限制&#10;D:\\Scripts&#10;C:\\Tools"></textarea>
          <p class="muted tiny">限制脚本和计划只能运行这些目录下的文件；留空表示允许本机任意路径。</p>
        </div>
        <div class="info-box">应用不会默认以管理员权限启动脚本，也不会安装解释器或修改 PATH。触发器动作只允许启动、停止、重启、通知和启停触发器。</div>
      </section>

      <section class="settings-card">
        <h3>界面</h3>
        <div class="field">
          <label class="label" for="language">语言</label>
          <select id="language" class="select" value={values.language ?? "zh-CN"} onchange={(event) => setValue("language", event.currentTarget.value)}>
            <option value="zh-CN">中文（简体）</option>
            <option value="en-US">English</option>
          </select>
          <p class="muted tiny">首版界面以中文为主，语言偏好会保存，供后续完整本地化使用。</p>
        </div>
        <div class="info-box">窗口关闭即销毁 WebView；托盘常驻时没有 WebView 内存。面板和主窗口不要同时长期打开。</div>
      </section>

      <section class="settings-card">
        <h3>关于与更新</h3>
        <div class="about-row">
          <div>
            <div class="about-name">LiteScriptManager <span class="version-badge">v{appVersion || updateInfo?.current_version || "—"}</span></div>
            <div class="muted tiny">更新来自 GitHub Releases（Jaspersol/LiteScriptManager）</div>
          </div>
          <button class="btn btn-sm" disabled={checking || installing} onclick={checkUpdate}>
            <Icon name="refresh" size={14} /> {checking ? "检查中…" : "检查更新"}
          </button>
        </div>
        <label class="setting-check">
          <input type="checkbox" checked={bool("auto_check_update")} onchange={(event) => setBool("auto_check_update", event.currentTarget.checked)} /> 启动时自动检查更新（每 6 小时最多一次）
        </label>

        {#if installing}
          <div class="update-box">
            <div class="update-line"><span class="status-dot running"></span> 
              {percent === null ? "正在下载更新包…" : percent >= 100 ? "下载完成，正在安装…" : `正在下载更新包 ${percent}%`}
            </div>
            <div class="progress"><div class="progress-bar" style:width={`${percent ?? 8}%`}></div></div>
            <div class="muted tiny">安装程序启动后应用会自动退出，并在安装完成后重新打开。</div>
          </div>
        {:else if updateError}
          <div class="update-box error">{updateError}</div>
        {:else if updateInfo && updateInfo.has_update}
          <div class="update-box">
            <div class="update-line"><span class="status-dot running"></span> 发现新版本 v{updateInfo.latest_version}
              {#if formatDate(updateInfo.published_at)}<span class="muted tiny">· {formatDate(updateInfo.published_at)}</span>{/if}
            </div>
            {#if updateInfo.notes}
              <div class="release-notes">{updateInfo.notes}</div>
            {/if}
            <div class="update-actions">
              <button class="btn btn-sm btn-primary" disabled={!updateInfo.download_url} onclick={installUpdate}>
                <Icon name="download" size={14} /> 下载并安装
              </button>
              {#if updateInfo.release_url}
                <button class="btn btn-sm btn-ghost" onclick={openReleasePage}>在 GitHub 查看</button>
              {/if}
              {#if updateInfo.asset_size}<span class="muted tiny">{formatBytes(updateInfo.asset_size)}</span>{/if}
            </div>
            {#if installError}
              <div class="install-error">{installError}<br />下载失败时可直接从 GitHub Release 页面手动下载安装包。</div>
            {/if}
          </div>
        {:else if updateInfo}
          <div class="update-box">当前已是最新版本 v{updateInfo.current_version}</div>
        {:else}
          <div class="info-box">点击「检查更新」从 GitHub Releases 获取最新版本；下载完成后会以静默方式安装并重启应用。</div>
        {/if}
      </section>
    </div>
  {/if}
</div>

<style>
  .settings-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 12px; }
  .settings-card { border: 1px solid var(--line-soft); background: #fff; border-radius: 16px; padding: 19px; }
  .settings-card h3 { margin: 0 0 16px; font-size: 12.5px; color: #3c434d; letter-spacing: .01em; font-weight: 550; }
  .setting-check { display: flex; align-items: flex-start; gap: 9px; color: #626a76; font-size: 12.5px; line-height: 1.5; margin-bottom: 13px; }
  .setting-check input { accent-color: var(--accent); margin-top: 2px; }
  .info-box { border: 1px solid var(--line-soft); background: #f8f9fb; border-radius: 11px; padding: 11px 12px; color: #7f8793; font-size: 11px; line-height: 1.6; }
  .field { margin-bottom: 13px; }
  .textarea { min-height: 82px; font-size: 11.5px; }
  .about-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 14px; }
  .about-name { font-size: 13px; color: #3c434d; font-weight: 550; display: flex; align-items: center; gap: 7px; }
  .version-badge { border: 1px solid var(--line-soft); background: #f8f9fb; border-radius: 999px; padding: 1px 8px; font-size: 11px; color: #6b7380; font-weight: 500; }
  .update-box { border: 1px solid var(--line-soft); background: #f8f9fb; border-radius: 11px; padding: 11px 12px; font-size: 11.5px; color: #5d6572; line-height: 1.6; }
  .update-box.error { border-color: #f2dddd; background: #fffafa; color: #a75757; }
  .update-line { display: flex; align-items: center; gap: 7px; flex-wrap: wrap; font-weight: 500; color: #3c434d; }
  .release-notes { margin-top: 8px; max-height: 132px; overflow: auto; white-space: pre-wrap; word-break: break-word; border-top: 1px dashed var(--line-soft); padding-top: 8px; color: #6b7380; font-size: 11px; }
  .update-actions { display: flex; align-items: center; gap: 8px; margin-top: 10px; flex-wrap: wrap; }
  .install-error { margin-top: 9px; border-top: 1px dashed #f2dddd; padding-top: 8px; color: #a75757; font-size: 11px; line-height: 1.6; }
  .progress { height: 6px; border-radius: 999px; background: #e6eaf1; overflow: hidden; margin: 9px 0 7px; }
  .progress-bar { height: 100%; background: var(--accent); border-radius: 999px; transition: width .2s ease; }
</style>
