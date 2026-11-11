// Dev-only: lets the UI run in a normal browser (`npm run dev`) with fake backend data.
// Never bundled into the desktop app (imported only when Tauri is absent and in dev mode).
import { mockIPC } from "@tauri-apps/api/mocks";
import fixture from "./fixture.json";
import type { FileItem, Filter, Sort } from "../types";

const items = fixture as unknown as FileItem[];
let running = false;
let found = 0;

function key(it: FileItem) {
  return "/" + it.source.replace(/\//g, "∕") + (it.dir === "/" ? "" : it.dir);
}

function match(it: FileItem, f: Filter, ignoreFolder = false) {
  if (f.text && !it.name.toLowerCase().includes(f.text.toLowerCase().replace(/\*/g, ""))) return false;
  if (f.categories.length && !f.categories.includes(it.category)) return false;
  if (f.methods.length && !f.methods.includes(it.method)) return false;
  if (f.scores.length && !f.scores.includes(it.score)) return false;
  if (f.exts.length && !f.exts.includes(it.ext)) return false;
  if (!ignoreFolder && f.folder && !(key(it) === f.folder || key(it).startsWith(f.folder + "/"))) return false;
  return true;
}

function visible() {
  return running ? items.slice(0, found) : items;
}

mockIPC((cmd, args: any) => {
  switch (cmd) {
    case "list_devices":
      return [
        { path: "/dev/disk3", name: "Macintosh HD, Macintosh HD - Data", kind: "container", size: 494384795648, fs: "APFS", mounts: ["/", "/System/Volumes/Data"], internal: true, removable: false, ssd: true, parent: "/dev/disk0" },
        { path: "/dev/disk4", name: "SanDisk Ultra", kind: "disk", size: 63864569856, fs: "FDisk_partition_scheme", mounts: [], internal: false, removable: true, ssd: false, parent: null },
        { path: "/dev/disk4s1", name: "CAMERA", kind: "partition", size: 63863521280, fs: "Windows_FAT_32", mounts: ["/Volumes/CAMERA"], internal: false, removable: true, ssd: false, parent: "/dev/disk4" },
      ];
    case "privileges":
      return { platform: "macos", elevated: false, hint: "Raw disk scans will ask for your administrator password. For the Trash and Time Machine, give Restorix Full Disk Access in System Settings › Privacy & Security." };
    case "start_scan":
      running = true;
      found = 0;
      return null;
    case "scan_progress": {
      if (running) found = Math.min(items.length, found + 3);
      if (found >= items.length) running = false;
      return { running, phase: running ? "FAT32: reading directories" : "Finished", done: found, total: items.length, found, messages: ["SanDisk Ultra: NTFS change journal not found or empty; deletion dates unavailable."], error: null, started: 0, finished: null };
    }
    case "query_items": {
      const f: Filter = args.filter;
      const s: Sort = args.sort;
      let v = visible().filter((i) => match(i, f));
      const k = (i: FileItem): any => (s.field === "name" ? i.name : s.field === "size" ? i.size : s.field === "deleted" ? i.deleted ?? i.deleted_after ?? -1 : s.field === "created" ? i.created ?? -1 : i.name);
      v = [...v].sort((a, b) => (k(a) > k(b) ? 1 : k(a) < k(b) ? -1 : 0));
      if (s.desc) v.reverse();
      return { total: v.length, totalSize: v.reduce((a, b) => a + b.size, 0), items: v.slice(args.offset, args.offset + args.limit) };
    }
    case "folder_tree": {
      const counts = new Map<string, number>();
      for (const it of visible().filter((i) => match(i, args.filter, true))) {
        let acc = "";
        for (const part of key(it).split("/").filter(Boolean)) {
          acc += "/" + part;
          counts.set(acc, (counts.get(acc) ?? 0) + 1);
        }
      }
      return [...counts.entries()].sort().map(([path, count]) => ({ path, name: path.split("/").pop(), depth: path.split("/").length - 2, count }));
    }
    case "scan_stats": {
      const by = (f: (i: FileItem) => string) => visible().reduce((m: Record<string, number>, i) => ((m[f(i)] = (m[f(i)] ?? 0) + 1), m), {});
      const exts = Object.entries(by((i) => i.ext)).sort((a, b) => b[1] - a[1]);
      return { total: visible().length, byCategory: by((i) => i.category), byMethod: by((i) => i.method), byScore: by((i) => i.score), exts };
    }
    case "preview_item": {
      const it = items.find((i) => i.id === args.id)!;
      if (it.category === "image") {
        const svg = `<svg xmlns='http://www.w3.org/2000/svg' width='600' height='400'><defs><linearGradient id='g' x1='0' x2='1' y1='0' y2='1'><stop offset='0' stop-color='#4aa3f0'/><stop offset='1' stop-color='#f3c26b'/></linearGradient></defs><rect width='600' height='400' fill='url(#g)'/><circle cx='460' cy='110' r='50' fill='#fff8'/><path d='M0 400 L180 220 L300 330 L420 240 L600 400Z' fill='#2d6a4f'/></svg>`;
        return { kind: "image", dataUrl: "data:image/svg+xml;base64," + btoa(svg), path: null, text: null, note: null, zeroRatio: 0 };
      }
      return { kind: "text", dataUrl: null, path: null, text: "Meeting notes\nMeeting notes\nMeeting notes\n", note: null, zeroRatio: 0 };
    }
    case "check_destination":
      return { sameDisk: String(args.dest).startsWith("/Volumes/CAMERA"), exists: true, message: "This folder is on the disk being recovered. Choose a folder on another drive." };
    case "start_restore":
      return null;
    case "restore_progress":
      return { running: false, done: 3, total: 3, bytes: 1153893, ok: 3, failed: 0, errors: [], dest: "/Volumes/Backup/Recovered", report: null };
    case "plugin:dialog|open":
      return args?.options?.directory ? "/Volumes/Backup/Recovered" : "/Users/me/Desktop/card.img";
    default:
      return null;
  }
});
