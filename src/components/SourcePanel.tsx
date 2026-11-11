import { open } from "@tauri-apps/plugin-dialog";
import { fmtSize } from "../api";
import type { Device, Privileges, Progress, ScanOptions } from "../types";
import { IconDisk, IconImage, IconPlay, IconRefresh, IconStop, IconUsb, IconWarn } from "./Icons";

const CARVE_TYPES: [string, string][] = [
  ["jpg", "JPEG"],
  ["png", "PNG"],
  ["heic", "HEIC / AVIF"],
  ["raw", "Camera RAW"],
  ["tif", "TIFF"],
  ["gif", "GIF"],
  ["bmp", "BMP"],
  ["webp", "WebP"],
  ["mp4", "MP4 / M4V"],
  ["mov", "MOV"],
  ["avi", "AVI"],
  ["3gp", "3GP"],
];

interface Props {
  devices: Device[];
  priv: Privileges | null;
  selected: Device | null;
  onSelect: (d: Device) => void;
  onRefresh: () => void;
  opts: ScanOptions;
  setOpts: (o: ScanOptions) => void;
  progress: Progress | null;
  onStart: () => void;
  onCancel: () => void;
}

function deviceIcon(d: Device) {
  if (d.kind === "image") return <IconImage />;
  return d.removable ? <IconUsb /> : <IconDisk />;
}

export default function SourcePanel(p: Props) {
  const running = !!p.progress?.running;
  const groups: { title: string; list: Device[] }[] = [
    { title: "Drives & volumes", list: p.devices.filter((d) => d.kind !== "image" && (d.kind !== "disk" || !p.devices.some((x) => x.parent === d.path))) },
    { title: "Whole disks", list: p.devices.filter((d) => d.kind === "disk" && p.devices.some((x) => x.parent === d.path)) },
  ];

  async function pickImage() {
    const f = await open({ multiple: false, directory: false, title: "Open a disk image", filters: [{ name: "Disk images", extensions: ["img", "dd", "raw", "dmg", "iso", "bin", "001", "vhd"] }, { name: "All files", extensions: ["*"] }] });
    if (typeof f === "string") {
      const name = f.split(/[\\/]/).pop() || f;
      p.onSelect({ path: f, name, kind: "image", size: 0, fs: "Image", mounts: [], internal: false, removable: false, ssd: null, parent: null });
    }
  }

  const set = (k: keyof ScanOptions, v: boolean) => p.setOpts({ ...p.opts, [k]: v });
  const toggleType = (t: string) => {
    const s = new Set(p.opts.carveTypes);
    s.has(t) ? s.delete(t) : s.add(t);
    p.setOpts({ ...p.opts, carveTypes: [...s] });
  };
  const sel = p.selected;
  const ssdWarn = sel && sel.ssd && sel.internal;
  const pct = p.progress && p.progress.total > 0 ? Math.min(100, (p.progress.done / p.progress.total) * 100) : null;

  return (
    <aside className="sidebar">
      <div className="brand">
        <img src="/icon.png" alt="" />
        <div>
          <div className="brand-name">Restorix</div>
          <div className="brand-sub">Deleted file recovery</div>
        </div>
      </div>

      {p.priv && !p.priv.elevated && (
        <div className="notice info">
          <IconWarn />
          <span>{p.priv.hint}</span>
        </div>
      )}

      <div className="section-head">
        <span>1. Choose what to scan</span>
        <button className="icon-btn" title="Refresh drive list" onClick={p.onRefresh} disabled={running}>
          <IconRefresh />
        </button>
      </div>
      <div className="device-list">
        {groups.map(
          (g) =>
            g.list.length > 0 && (
              <div key={g.title}>
                <div className="group-title">{g.title}</div>
                {g.list.map((d) => (
                  <button key={d.path} className={"device" + (sel?.path === d.path ? " active" : "")} onClick={() => p.onSelect(d)} disabled={running}>
                    <span className="device-icon">{deviceIcon(d)}</span>
                    <span className="device-text">
                      <span className="device-name">{d.name}</span>
                      <span className="device-meta">
                        {[d.size ? fmtSize(d.size) : "", d.fs, d.mounts[0], d.ssd ? "SSD" : d.ssd === false ? "HDD" : ""].filter(Boolean).join(" · ")}
                      </span>
                    </span>
                  </button>
                ))}
              </div>
            ),
        )}
        {p.devices.length === 0 && <div className="muted small pad">No drives found yet.</div>}
        {sel?.kind === "image" && (
          <button className="device active">
            <span className="device-icon">
              <IconImage />
            </span>
            <span className="device-text">
              <span className="device-name">{sel.name}</span>
              <span className="device-meta">Disk image</span>
            </span>
          </button>
        )}
        <button className="btn ghost wide" onClick={pickImage} disabled={running}>
          <IconImage /> Open a disk image…
        </button>
      </div>

      {ssdWarn && (
        <div className="notice warn">
          <IconWarn />
          <span>
            This is an internal SSD. TRIM usually erases deleted data within minutes, so file-system and carving results may be empty or zeroed. The Trash and Snapshots are your best chance here.
          </span>
        </div>
      )}

      <div className="section-head">
        <span>2. Recovery methods</span>
      </div>
      <div className="options">
        <label className="check">
          <input type="checkbox" checked={p.opts.trash} onChange={(e) => set("trash", e.target.checked)} disabled={running} />
          <span>
            <b>Trash / Recycle Bin</b>
            <small>Items still in the bin, with original location and deletion date</small>
          </span>
        </label>
        <label className="check">
          <input type="checkbox" checked={p.opts.snapshots} onChange={(e) => set("snapshots", e.target.checked)} disabled={running} />
          <span>
            <b>Snapshots &amp; backups</b>
            <small>Shadow Copies, APFS snapshots, Time Machine. Works on SSDs.</small>
          </span>
        </label>
        <label className="check">
          <input type="checkbox" checked={p.opts.metadata} onChange={(e) => set("metadata", e.target.checked)} disabled={running} />
          <span>
            <b>File system scan</b>
            <small>NTFS, FAT, exFAT, HFS+, APFS. Keeps names, folders and dates.</small>
          </span>
        </label>
        <label className="check">
          <input type="checkbox" checked={p.opts.carving} onChange={(e) => set("carving", e.target.checked)} disabled={running} />
          <span>
            <b>Deep scan (carving)</b>
            <small>Finds photos &amp; videos by signature, even without metadata</small>
          </span>
        </label>
        {p.opts.carving && (
          <div className="carve-opts">
            <label className="check slim">
              <input type="checkbox" checked={p.opts.carveFreeOnly} onChange={(e) => set("carveFreeOnly", e.target.checked)} disabled={running} />
              <span>Free space only (faster)</span>
            </label>
            <div className="chips">
              {CARVE_TYPES.map(([t, l]) => (
                <button key={t} className={"chip" + (p.opts.carveTypes.includes(t) ? " on" : "")} onClick={() => toggleType(t)} disabled={running}>
                  {l}
                </button>
              ))}
            </div>
          </div>
        )}
      </div>

      <div className="scan-box">
        {running ? (
          <>
            <div className="phase">{p.progress?.phase || "Starting…"}</div>
            <div className="bar">
              <div className={"bar-fill" + (pct == null ? " indet" : "")} style={{ width: pct == null ? "35%" : `${pct}%` }} />
            </div>
            <div className="scan-stats">
              <span>{p.progress?.found ?? 0} files found</span>
              {pct != null && <span>{pct.toFixed(0)}%</span>}
            </div>
            <button className="btn danger wide" onClick={p.onCancel}>
              <IconStop /> Stop scan
            </button>
          </>
        ) : (
          <button className="btn primary wide big" onClick={p.onStart} disabled={!sel || !(p.opts.trash || p.opts.snapshots || p.opts.metadata || p.opts.carving)}>
            <IconPlay /> {sel ? `Scan ${sel.name}` : "Select a drive to scan"}
          </button>
        )}
      </div>
    </aside>
  );
}
