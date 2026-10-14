// Types mirroring the JSON produced by the daemon (schema_version 1).

export type Kind = "activity" | "status";

export interface Resolved {
  value: string;
  source: "own" | "inherited";
}

export interface Favorite {
  id: string;
  title: string;
  path: string[];
  type: "folder" | "leaf";
  kind?: Kind;
  color: Resolved | null;
  icon: Resolved | null;
  file: string;
  line: number;
  generated: boolean;
  children?: Favorite[];
}

export interface RunningInfo {
  id: string;
  title: string;
  path: string[];
  kind: Kind;
  origin: "config" | "deck" | "adhoc";
  note: string | null;
  source: string;
  start: string;
  elapsed: number;
  color: string | null;
  icon: string | null;
}

export interface StoppedInfo {
  id: string;
  title: string;
  path: string[];
  kind: Kind;
  origin: string;
  note: string | null;
  ended: string;
}

export interface ServerInfo {
  url: string;
  bucket: string;
  online: boolean | null;
  pending: number;
  failed: number;
  last_error: string | null;
}

export interface Status {
  schema_version: number;
  running: RunningInfo | null;
  last_stopped: StoppedInfo | null;
  server: ServerInfo;
  now: string;
}

export interface ScrollConfig {
  layout: "tree" | "levels";
  confirm_with_hotkey: boolean;
  dwell_confirm: boolean;
  dwell_time: string;
  timeout: string;
  sensitivity: number;
  wrap_around: boolean;
  open_at_cursor: boolean;
  start_at: "running" | "first";
  horizontal: boolean;
}

export type ElapsedFormat = "1h 23m" | "1:23" | "1h 23m 45s" | "1:23:45";
export type TimeFormat = "24h" | "12h";

export interface Snapshot {
  status: Status;
  favorites: Favorite[];
  scroll: ScrollConfig;
  tray: { elapsed_format: ElapsedFormat; time_format: TimeFormat };
}

export type NavCommand = "next" | "prev" | "confirm" | "back" | "cancel";
