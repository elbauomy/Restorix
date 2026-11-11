export type Category = "image" | "video" | "audio" | "document" | "archive" | "other";
export type Method = "trash" | "snapshot" | "metadata" | "carved";
export type Score = "good" | "partial" | "overwritten" | "unknown";

export interface Device {
  path: string;
  name: string;
  kind: "disk" | "partition" | "volume" | "container" | "image";
  size: number;
  fs: string;
  mounts: string[];
  internal: boolean;
  removable: boolean;
  ssd: boolean | null;
  parent: string | null;
}

export interface Privileges {
  platform: "macos" | "windows" | "other";
  elevated: boolean;
  hint: string;
}

export interface FileItem {
  id: number;
  name: string;
  dir: string;
  ext: string;
  category: Category;
  size: number;
  created: number | null;
  modified: number | null;
  deleted: number | null;
  deleted_after: number | null;
  method: Method;
  fs: string;
  source: string;
  score: Score;
  note: string;
}

export interface Progress {
  running: boolean;
  phase: string;
  done: number;
  total: number;
  found: number;
  messages: string[];
  error: string | null;
  started: number;
  finished: number | null;
}

export interface Filter {
  text: string;
  categories: Category[];
  exts: string[];
  methods: Method[];
  scores: Score[];
  deletedFrom: number | null;
  deletedTo: number | null;
  createdFrom: number | null;
  createdTo: number | null;
  includeUnknownDates: boolean;
  sizeMin: number | null;
  sizeMax: number | null;
  folder: string | null;
}

export interface Sort {
  field: string;
  desc: boolean;
}

export interface Page {
  total: number;
  totalSize: number;
  items: FileItem[];
}

export interface FolderNode {
  path: string;
  name: string;
  depth: number;
  count: number;
}

export interface Stats {
  total: number;
  byCategory: Record<string, number>;
  byMethod: Record<string, number>;
  byScore: Record<string, number>;
  exts: [string, number][];
}

export interface Preview {
  kind: "image" | "video" | "text" | "hex" | "none";
  dataUrl: string | null;
  path: string | null;
  text: string | null;
  note: string | null;
  zeroRatio: number;
}

export interface RestoreProgress {
  running: boolean;
  done: number;
  total: number;
  bytes: number;
  ok: number;
  failed: number;
  errors: string[];
  dest: string;
  report: string | null;
}

export interface ScanOptions {
  trash: boolean;
  snapshots: boolean;
  metadata: boolean;
  carving: boolean;
  carveFreeOnly: boolean;
  carveTypes: string[];
}

export const emptyFilter: Filter = {
  text: "",
  categories: [],
  exts: [],
  methods: [],
  scores: [],
  deletedFrom: null,
  deletedTo: null,
  createdFrom: null,
  createdTo: null,
  includeUnknownDates: true,
  sizeMin: null,
  sizeMax: null,
  folder: null,
};
