export type RunStatus = "queued" | "starting" | "running" | "stopping" | "stopped" | "exited" | "failed";

export interface Script {
  id: string;
  name: string;
  path: string;
  runtime_id: string;
  args: string[];
  cwd: string | null;
  env: Record<string, string>;
  interpreter_path: string | null;
  process_match: string | null;
  stop_timeout_sec: number;
  timeout_sec: number | null;
  stop_mode: string;
  max_instances: number;
  instance_policy: string;
  success_codes: string;
  content_hash: string | null;
  sort_order: number;
  enabled: boolean;
  created_at: number;
  updated_at: number;
  status: RunStatus;
  last_error: string | null;
  last_run_id: string | null;
}

export interface ScriptEdit {
  name: string;
  args: string[];
  cwd: string | null;
  interpreter_path: string | null;
  process_match: string | null;
  stop_timeout_sec: number;
  timeout_sec: number | null;
  stop_mode: string;
  max_instances: number;
  instance_policy: string;
  success_codes: string;
  enabled: boolean;
}

export interface RuntimeCandidate {
  path: string;
  version: string | null;
  source: string;
  kind: string;
}

export interface RuntimeInfo {
  id: string;
  name: string;
  installed: boolean;
  exe_path: string | null;
  version: string | null;
  status: string;
  candidates: RuntimeCandidate[];
  error: string | null;
  details: string[];
}

export interface RuntimeMeta {
  id: string;
  name: string;
  extensions: string[];
}

export interface RunStateEvent {
  run_id: string;
  script_id: string;
  status: RunStatus;
  exit_code: number | null;
  error: string | null;
  started_at: number;
  ended_at: number | null;
  source: string | null;
  source_id: string | null;
}

export interface LogLine {
  stream: "stdout" | "stderr";
  line: string;
}

export interface LogBatchEvent {
  run_id: string;
  script_id: string;
  lines: LogLine[];
}

export interface Schedule {
  id: string;
  name: string;
  script_id: string;
  kind: string;
  expression: string;
  timezone: string;
  enabled: boolean;
  overlap_policy: string;
  catch_up: string;
  timeout_sec: number | null;
  success_codes: string;
  restart_delay_sec: number;
  max_restarts: number;
  backoff: boolean;
  next_run_at: number | null;
  last_run_at: number | null;
  last_status: string | null;
  last_error: string | null;
  last_run_id: string | null;
  created_at: number;
  updated_at: number;
}

export interface ScheduleEdit {
  name: string;
  script_id: string;
  kind: string;
  expression: string;
  timezone: string;
  enabled: boolean;
  overlap_policy: string;
  catch_up: string;
  timeout_sec: number | null;
  success_codes: string;
  restart_delay_sec: number;
  max_restarts: number;
  backoff: boolean;
}

export interface SchedulePreview {
  next: number[];
  error: string | null;
}

export interface Trigger {
  id: string;
  name: string;
  enabled: boolean;
  event_type: string;
  config_json: string;
  actions_json: string;
  cooldown_sec: number;
  last_triggered_at: number | null;
  last_result: string | null;
  created_at: number;
  updated_at: number;
}

export interface TriggerEdit {
  name: string;
  enabled: boolean;
  event_type: string;
  config_json: string;
  actions_json: string;
  cooldown_sec: number;
}

export interface TriggerEvent {
  id: string;
  trigger_id: string;
  trigger_name: string;
  fired_at: number;
  event: string;
  result: string;
}

export interface RunRecord {
  id: string;
  script_id: string;
  script_name: string;
  status: string;
  pid: number | null;
  started_at: number;
  ended_at: number | null;
  exit_code: number | null;
  log_path: string | null;
  error: string | null;
  source: string;
  source_id: string | null;
  stop_reason: string | null;
  peak_memory_bytes: number | null;
  cpu_time_ms: number | null;
  timeout_sec: number | null;
  log_bytes: number | null;
}

export type Settings = Record<string, string>;
