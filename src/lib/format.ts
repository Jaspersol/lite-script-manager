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

const EXIT_CODE_LIMIT = 0xffffffff;

function parseExitCodeToken(token: string): number | null {
  const value = token.trim();
  if (!value) return null;
  if (/^0x[0-9a-f]+$/i.test(value)) {
    const parsed = Number.parseInt(value.slice(2), 16);
    return Number.isNaN(parsed) || parsed > EXIT_CODE_LIMIT ? null : parsed;
  }
  if (!/^\d+$/.test(value)) return null;
  const parsed = Number(value);
  return parsed > EXIT_CODE_LIMIT ? null : parsed;
}

/**
 * Accepts `0`, `0,2,10-12`, `0xC0000005` and ranges of either form. Windows
 * exit codes above 2^31 are unsigned, so decimal values up to 4294967295 are valid.
 */
export function isValidSuccessCodes(input: string): boolean {
  const text = input.trim();
  if (!text || text === "*") return true;
  return text.split(",").every((part) => {
    const token = part.trim();
    if (!token) return false;
    const dash = token.indexOf("-");
    if (dash > 0) {
      const start = parseExitCodeToken(token.slice(0, dash));
      const end = parseExitCodeToken(token.slice(dash + 1));
      return start !== null && end !== null && start <= end;
    }
    return parseExitCodeToken(token) !== null;
  });
}

/** Shows the unsigned exit code and its hex form for Windows NTSTATUS values. */
export function exitCodeText(code: number | null | undefined): string {
  if (code == null) return "—";
  const value = code >>> 0;
  if (value < 0x80000000) return String(value);
  return `${value} (0x${value.toString(16).toUpperCase().padStart(8, "0")})`;
}
