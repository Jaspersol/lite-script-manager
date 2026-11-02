import { invoke } from "@tauri-apps/api/core";
import type { RuntimeInfo, RuntimeMeta, Script } from "./types";

export const api = {
  listScripts: () => invoke<Script[]>("list_scripts"),
  listRuntimeCatalog: () => invoke<RuntimeMeta[]>("list_runtime_catalog"),
  detectRuntimes: (force = false) => invoke<RuntimeInfo[]>("detect_runtimes", { force }),
  createFromPath: (path: string) => invoke<Script>("create_script_from_path", { path }),
  createNew: (name: string, runtimeId: string, content: string) =>
    invoke<Script>("create_new_script", { name, runtimeId, content }),
  updateScript: (id: string, name: string, args: string[], cwd: string | null, interpreterPath: string | null, stopTimeoutSec: number, enabled: boolean) =>
    invoke<Script>("update_script", { id, name, args, cwd, interpreterPath, stopTimeoutSec, enabled }),
  deleteScript: (id: string, deleteFile = false) =>
    invoke<void>("delete_script", { id, deleteFile }),
  reorderScripts: (ids: string[]) => invoke<void>("reorder_scripts", { ids }),
  runScript: (id: string) => invoke<unknown>("run_script", { id }),
  stopScript: (id: string) => invoke<void>("stop_script", { id }),
  stopAllScripts: () => invoke<void>("stop_all_scripts"),
  closePanel: () => invoke<void>("close_panel"),
  showMain: () => invoke<void>("show_main"),
  getRunLog: (runId: string, maxLines = 400) =>
    invoke<string[]>("get_run_log", { runId, maxLines }),
};

export function friendlyError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
