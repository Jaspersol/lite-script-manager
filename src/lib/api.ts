import { invoke } from "@tauri-apps/api/core";
import type {
  RunRecord,
  RunStateEvent,
  RuntimeInfo,
  RuntimeMeta,
  Schedule,
  ScheduleEdit,
  SchedulePreview,
  Script,
  ScriptEdit,
  Settings,
  Trigger,
  TriggerEdit,
  TriggerEvent,
  UpdateInfo,
} from "./types";

export const api = {
  listScripts: () => invoke<Script[]>("list_scripts"),
  listRuntimeCatalog: () => invoke<RuntimeMeta[]>("list_runtime_catalog"),
  detectRuntimes: (force = false) => invoke<RuntimeInfo[]>("detect_runtimes", { force }),
  createFromPath: (path: string) => invoke<Script>("create_script_from_path", { path }),
  createNew: (name: string, runtimeId: string, content: string) =>
    invoke<Script>("create_new_script", { name, runtimeId, content }),
  updateScript: (id: string, edit: ScriptEdit) =>
    invoke<Script>("update_script", { id, edit }),
  pinScriptHash: (id: string, pinned: boolean) =>
    invoke<string | null>("pin_script_hash", { id, pinned }),
  deleteScript: (id: string, deleteFile = false) =>
    invoke<void>("delete_script", { id, deleteFile }),
  reorderScripts: (ids: string[]) => invoke<void>("reorder_scripts", { ids }),
  runScript: (id: string) => invoke<RunStateEvent>("run_script", { id }),
  stopScript: (id: string) => invoke<void>("stop_script", { id }),
  stopAllScripts: () => invoke<void>("stop_all_scripts"),
  adoptExternalRuns: () => invoke<number>("adopt_external_runs"),
  setLogStreaming: (visible: boolean) => invoke<void>("set_log_streaming", { visible }),
  closePanel: () => invoke<void>("close_panel"),
  showMain: () => invoke<void>("show_main"),
  getRunLog: (runId: string, maxLines = 400) =>
    invoke<string[]>("get_run_log", { runId, maxLines }),
  openLogFile: (runId: string) => invoke<void>("open_log_file", { runId }),
  openScriptInEditor: (id: string) => invoke<void>("open_script_in_editor", { id }),
  listRuns: (filters: { scriptId?: string | null; status?: string | null; source?: string | null; limit?: number } = {}) =>
    invoke<RunRecord[]>("list_runs", {
      scriptId: filters.scriptId ?? null,
      status: filters.status ?? null,
      source: filters.source ?? null,
      limit: filters.limit ?? 200,
    }),
  clearRuns: (beforeMs?: number | null) => invoke<number>("clear_runs", { beforeMs: beforeMs ?? null }),
  listSchedules: () => invoke<Schedule[]>("list_schedules"),
  createSchedule: (edit: ScheduleEdit) => invoke<Schedule>("create_schedule", { edit }),
  updateSchedule: (id: string, edit: ScheduleEdit) => invoke<Schedule>("update_schedule", { id, edit }),
  deleteSchedule: (id: string) => invoke<void>("delete_schedule", { id }),
  runScheduleNow: (id: string) => invoke<string>("run_schedule_now", { id }),
  previewSchedule: (kind: string, expression: string, timezone = "local", count = 5) =>
    invoke<SchedulePreview>("preview_schedule", { kind, expression, timezone, count }),
  listTriggers: () => invoke<Trigger[]>("list_triggers"),
  createTrigger: (edit: TriggerEdit) => invoke<Trigger>("create_trigger", { edit }),
  updateTrigger: (id: string, edit: TriggerEdit) => invoke<Trigger>("update_trigger", { id, edit }),
  deleteTrigger: (id: string) => invoke<void>("delete_trigger", { id }),
  triggerNow: (id: string) => invoke<string>("trigger_now", { id }),
  listTriggerEvents: (limit = 100) => invoke<TriggerEvent[]>("list_trigger_events", { limit }),
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (values: Settings) => invoke<Settings>("update_settings", { values }),
  checkUpdate: () => invoke<UpdateInfo>("check_update"),
  lastUpdateResult: () => invoke<UpdateInfo | null>("last_update_result"),
  installUpdate: () => invoke<void>("install_update"),
  openReleasePage: (url: string) => invoke<void>("open_release_page", { url }),
};

export function friendlyError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

export function formatTime(value: number | null | undefined): string {
  if (!value) return "—";
  return new Date(value).toLocaleString("zh-CN", { hour12: false });
}

export function formatDuration(start: number | null, end: number | null): string {
  if (!start) return "—";
  const seconds = Math.max(0, Math.round(((end ?? Date.now()) - start) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

export function formatBytes(value: number | null | undefined): string {
  if (!value) return "—";
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index++;
  }
  return `${size.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}
