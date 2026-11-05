import { t } from "./i18n.svelte";
import type { RunStateEvent, Script } from "./types";

const STATUS_LABELS: Record<string, string> = {
  stopped: "已停止",
  queued: "排队中",
  starting: "启动中",
  running: "运行中",
  stopping: "停止中",
  exited: "已结束",
  failed: "失败",
  skipped: "已跳过",
};

const SOURCE_LABELS: Record<string, string> = {
  manual: "手动",
  schedule: "定时",
  loop: "循环",
  trigger: "触发器",
  adopted: "接管",
  queue: "排队",
};

export function statusLabel(value: string | null | undefined): string {
  if (!value) return t("未运行");
  const label = STATUS_LABELS[value];
  return label ? t(label) : value;
}

export function sourceLabel(value: string | null | undefined): string {
  if (!value) return t("未知");
  const label = SOURCE_LABELS[value];
  return label ? t(label) : value;
}

export function isBusyStatus(status: string): boolean {
  return status === "starting" || status === "stopping" || status === "queued";
}

export function isActiveStatus(status: string): boolean {
  return status === "running" || status === "starting" || status === "queued";
}

/**
 * Applies a run-state event to a script. A `queued-*` placeholder must not
 * overwrite the state of a live instance, or the stop button stays disabled.
 */
export function applyRunState(script: Script, payload: RunStateEvent): Script {
  if (
    payload.run_id.startsWith("queued-") &&
    (script.status === "running" || script.status === "starting" || script.status === "stopping")
  ) {
    return script;
  }
  return { ...script, status: payload.status, last_error: payload.error, last_run_id: payload.run_id };
}

export function scriptName(scripts: Script[], id: string): string {
  return scripts.find((script) => script.id === id)?.name ?? t("已删除脚本");
}

export function formatLogLine(line: { stream: string; line: string }): string {
  return `[${line.stream}] ${line.line}`;
}
