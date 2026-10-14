import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { NavCommand, Snapshot } from "./types";

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  start: (id: string, note: string | null, source: "popover" | "popover-scroll") =>
    invoke("start_favorite", { id, note, source }),
  stop: (source: "popover" | "popover-scroll") => invoke("stop", { source }),
  setNote: (note: string | null) => invoke("set_note", { note }),
  log: (id: string, range: string, note: string | null) => invoke("log_entry", { id, range, note }),
  previewRange: (range: string) => invoke<{ start: string; end: string; text: string }>("preview_range", { range }),
  hide: () => invoke("hide_popover"),
  openSettings: () => invoke("open_settings"),
};

/** Why the popover was opened (payload of `popover:opened`). */
export interface Opened {
  trigger: string;
  /** Ask for a note for this favorite, then start it. */
  favorite?: string;
  /** Open the note field for the running activity. */
  note?: boolean;
}

export function onOpened(f: (opened: Opened) => void): Promise<UnlistenFn> {
  return listen<Opened>("popover:opened", (e) => f(e.payload));
}

export function onClosed(f: () => void): Promise<UnlistenFn> {
  return listen("popover:closed", () => f());
}

export function onNav(f: (cmd: NavCommand) => void): Promise<UnlistenFn> {
  return listen<NavCommand>("popover:nav", (e) => f(e.payload));
}

export function onStateChanged(f: () => void): Promise<UnlistenFn> {
  return listen("state-changed", () => f());
}

/** Error messages from commands are plain strings (or lists of strings). */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (Array.isArray(e)) return e.join("\n");
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
