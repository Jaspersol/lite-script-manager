export type RunStatus = "stopped" | "starting" | "running" | "stopping" | "exited" | "failed";

export interface Script {
  id: string;
  name: string;
  path: string;
  runtime_id: string;
  args: string[];
  cwd: string | null;
  env: Record<string, string>;
  sort_order: number;
  enabled: boolean;
  created_at: number;
  updated_at: number;
  status: RunStatus;
  last_error: string | null;
  last_run_id: string | null;
}

export interface RuntimeCandidate {
  path: string;
  version: string | null;
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
}

export interface LogEvent {
  run_id: string;
  script_id: string;
  stream: "stdout" | "stderr";
  line: string;
}
