<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, friendlyError } from "./api";
  import { pageAction } from "./uiActions";
  import type { PageAction } from "./uiActions";
  import type { Settings } from "./types";

  let {
    notify = () => {},
  }: {
    notify: (message: string, kind?: "info" | "error") => void;
  } = $props();

  let values: Settings = $state({});
  let loading = $state(true);
  let saving = $state(false);

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
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    (async () => {
      const off = await listen("settings-changed", load);
      if (disposed) off(); else unlisteners.push(off);
    })();
    return () => {
      disposed = true;
      unlisteners.forEach((fn) => fn());
    };
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
</style>
