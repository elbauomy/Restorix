import { invoke } from "@tauri-apps/api/core";
import type { Device, Filter, FolderNode, Page, Preview, Privileges, Progress, RestoreProgress, ScanOptions, Sort, Stats } from "./types";

export interface ScanRequest extends ScanOptions {
  source: string;
  label: string;
  mounts: string[];
}

export const api = {
  listDevices: () => invoke<Device[]>("list_devices"),
  privileges: () => invoke<Privileges>("privileges"),
  startScan: (req: ScanRequest) => invoke<void>("start_scan", { req }),
  scanProgress: () => invoke<Progress | null>("scan_progress"),
  cancelScan: () => invoke<void>("cancel_scan"),
  queryItems: (filter: Filter, sort: Sort, offset: number, limit: number) => invoke<Page>("query_items", { filter, sort, offset, limit }),
  folderTree: (filter: Filter) => invoke<FolderNode[]>("folder_tree", { filter }),
  stats: () => invoke<Stats>("scan_stats"),
  preview: (id: number) => invoke<Preview>("preview_item", { id }),
  checkDestination: (dest: string) => invoke<{ sameDisk: boolean; exists: boolean; message: string | null }>("check_destination", { dest }),
  startRestore: (args: { ids?: number[]; filter?: Filter; dest: string; keepStructure: boolean }) =>
    invoke<void>("start_restore", { ids: args.ids ?? null, filter: args.filter ?? null, dest: args.dest, keepStructure: args.keepStructure }),
  restoreProgress: () => invoke<RestoreProgress>("restore_progress"),
  cancelRestore: () => invoke<void>("cancel_restore"),
  exportCsv: (filter: Filter, path: string) => invoke<number>("export_csv", { filter, path }),
};

export function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  const u = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${u[i]}`;
}

export function fmtDate(t: number | null | undefined): string {
  if (t == null) return "";
  const d = new Date(t * 1000);
  return d.toLocaleString(undefined, { year: "numeric", month: "short", day: "2-digit", hour: "2-digit", minute: "2-digit" });
}

/** yyyy-mm-dd (local) -> unix seconds at start or end of that day */
export function dayToUnix(s: string, end: boolean): number | null {
  if (!s) return null;
  const [y, m, d] = s.split("-").map(Number);
  const dt = end ? new Date(y, m - 1, d, 23, 59, 59) : new Date(y, m - 1, d, 0, 0, 0);
  return Math.floor(dt.getTime() / 1000);
}

export function unixToDay(t: number | null): string {
  if (t == null) return "";
  const d = new Date(t * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

export const METHOD_LABEL: Record<string, string> = {
  trash: "Trash / Recycle Bin",
  snapshot: "Snapshot / Backup",
  metadata: "File system",
  carved: "Carved",
};

export const METHOD_SHORT: Record<string, string> = {
  trash: "Trash",
  snapshot: "Snapshot",
  metadata: "File system",
  carved: "Carved",
};

export function fmtShort(t: number | null | undefined): string {
  if (t == null) return "";
  return new Date(t * 1000).toLocaleString(undefined, { year: "2-digit", month: "short", day: "2-digit", hour: "2-digit", minute: "2-digit" });
}

export const SCORE_LABEL: Record<string, string> = {
  good: "Good",
  partial: "Partial",
  overwritten: "Overwritten",
  unknown: "Unknown",
};

export const CATEGORY_LABEL: Record<string, string> = {
  image: "Photos",
  video: "Videos",
  audio: "Audio",
  document: "Documents",
  archive: "Archives",
  other: "Other",
};
