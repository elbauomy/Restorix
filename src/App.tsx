import { save } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, fmtSize } from "./api";
import FilterBar from "./components/FilterBar";
import FolderTree from "./components/FolderTree";
import { IconExport, IconRestore, IconWarn } from "./components/Icons";
import PreviewPanel from "./components/PreviewPanel";
import ResultsTable from "./components/ResultsTable";
import RestoreDialog from "./components/RestoreDialog";
import SourcePanel from "./components/SourcePanel";
import { emptyFilter, type Device, type FileItem, type Filter, type FolderNode, type Privileges, type Progress, type ScanOptions, type Sort, type Stats } from "./types";

const PAGE = 300;

const defaultOpts: ScanOptions = {
  trash: true,
  snapshots: true,
  metadata: true,
  carving: true,
  carveFreeOnly: true,
  carveTypes: ["jpg", "png", "heic", "raw", "tif", "gif", "bmp", "webp", "mp4", "mov", "avi", "3gp"],
};

function useDebounced<T>(v: T, ms: number): T {
  const [d, setD] = useState(v);
  useEffect(() => {
    const t = setTimeout(() => setD(v), ms);
    return () => clearTimeout(t);
  }, [v, ms]);
  return d;
}

export default function App() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [priv, setPriv] = useState<Privileges | null>(null);
  const [selected, setSelected] = useState<Device | null>(null);
  const [opts, setOpts] = useState<ScanOptions>(defaultOpts);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [scanned, setScanned] = useState(false);

  const [filter, setFilter] = useState<Filter>(emptyFilter);
  const dFilter = useDebounced(filter, 200);
  const [sort, setSort] = useState<Sort>({ field: "deleted", desc: true });
  const [items, setItems] = useState<FileItem[]>([]);
  const [total, setTotal] = useState(0);
  const [totalSize, setTotalSize] = useState(0);
  const [limit, setLimit] = useState(PAGE);
  const [tree, setTree] = useState<FolderNode[]>([]);
  const [stats, setStats] = useState<Stats | null>(null);

  const [sel, setSel] = useState<Set<number>>(new Set());
  const [allFiltered, setAllFiltered] = useState(false);
  const [current, setCurrent] = useState<FileItem | null>(null);
  const [restoreReq, setRestoreReq] = useState<{ ids?: number[]; filter?: Filter; count: number; size: number | null } | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  const refreshDevices = useCallback(() => {
    api.listDevices().then(setDevices).catch((e) => setToast(String(e)));
  }, []);

  useEffect(() => {
    refreshDevices();
    api.privileges().then(setPriv);
  }, [refreshDevices]);

  const reload = useCallback(async () => {
    const [page, tr, st] = await Promise.all([api.queryItems(dFilter, sort, 0, limit), api.folderTree({ ...dFilter, folder: null }), api.stats()]);
    setItems(page.items);
    setTotal(page.total);
    setTotalSize(page.totalSize);
    setTree(tr);
    setStats(st);
  }, [dFilter, sort, limit]);

  useEffect(() => {
    if (scanned) reload();
  }, [reload, scanned]);

  // Poll progress while a scan runs; refresh results every couple of seconds.
  const tick = useRef(0);
  useEffect(() => {
    if (!progress?.running) return;
    const t = setInterval(async () => {
      const p = await api.scanProgress();
      setProgress(p);
      tick.current++;
      if (tick.current % 4 === 0 || !p?.running) reload();
    }, 500);
    return () => clearInterval(t);
  }, [progress?.running, reload]);

  useEffect(() => {
    setLimit(PAGE);
    setAllFiltered(false);
  }, [dFilter, sort]);

  async function startScan() {
    if (!selected) return;
    const mounts = selected.mounts.length ? selected.mounts : devices.filter((d) => d.parent === selected.path).flatMap((d) => d.mounts);
    try {
      await api.startScan({ source: selected.path, label: selected.name, mounts, ...opts });
      setScanned(true);
      setSel(new Set());
      setAllFiltered(false);
      setCurrent(null);
      setFilter(emptyFilter);
      setProgress(await api.scanProgress());
    } catch (e) {
      setToast(String(e));
    }
  }

  async function exportCsv() {
    const path = await save({ title: "Export results", defaultPath: "restorix-results.csv", filters: [{ name: "CSV", extensions: ["csv"] }] });
    if (path) {
      const n = await api.exportCsv(dFilter, path);
      setToast(`Exported ${n} rows to ${path}`);
    }
  }

  const selCount = allFiltered ? total : sel.size;
  const openRestore = () => {
    if (allFiltered) setRestoreReq({ filter: dFilter, count: total, size: totalSize });
    else {
      const ids = [...sel];
      const size = items.filter((i) => sel.has(i.id)).reduce((a, b) => a + b.size, 0);
      setRestoreReq({ ids, count: ids.length, size: ids.length === items.filter((i) => sel.has(i.id)).length ? size : null });
    }
  };

  return (
    <div className="app">
      <SourcePanel
        devices={devices}
        priv={priv}
        selected={selected}
        onSelect={setSelected}
        onRefresh={refreshDevices}
        opts={opts}
        setOpts={setOpts}
        progress={progress}
        onStart={startScan}
        onCancel={() => api.cancelScan()}
      />

      <main className="main">
        {!scanned ? (
          <div className="welcome">
            <img src="/icon.png" alt="" />
            <h1>Recover deleted files</h1>
            <p>Choose a drive, memory card or disk image on the left and press Scan. Restorix only reads the drive; nothing is changed until you restore files to another location.</p>
            <ul className="tips">
              <li>
                <b>Stop using the drive</b> that held the deleted files. New data can overwrite them.
              </li>
              <li>
                <b>Restore to a different drive</b>, such as a USB stick or external disk.
              </li>
              <li>
                <b>Internal SSDs</b> erase deleted data quickly (TRIM). Check the Trash and Snapshots first.
              </li>
            </ul>
          </div>
        ) : (
          <>
            <FilterBar filter={filter} setFilter={setFilter} stats={stats} />
            {progress && (progress.messages.length > 0 || progress.error) && (
              <div className="messages">
                {progress.error && (
                  <div className="notice danger">
                    <IconWarn />
                    <span>{progress.error}</span>
                  </div>
                )}
                {progress.messages.map((m) => (
                  <div className="notice info" key={m}>
                    <IconWarn />
                    <span>{m}</span>
                  </div>
                ))}
              </div>
            )}
            <div className={"content" + (current ? " with-preview" : "")}>
              <FolderTree nodes={tree} selected={filter.folder} onSelect={(f) => setFilter({ ...filter, folder: f })} total={stats?.total ?? 0} />
              <div className="results-col">
                {total === 0 ? (
                  <div className="empty">{progress?.running ? "Scanning… files will appear here as they are found." : "No deleted files match these filters."}</div>
                ) : (
                  <ResultsTable
                    items={items}
                    sort={sort}
                    setSort={setSort}
                    selected={sel}
                    setSelected={setSel}
                    allFiltered={allFiltered}
                    setAllFiltered={setAllFiltered}
                    current={current?.id ?? null}
                    onOpen={setCurrent}
                    onMore={() => setLimit(limit + PAGE)}
                    hasMore={items.length < total}
                    total={total}
                  />
                )}
              </div>
              {current && <PreviewPanel item={current} onClose={() => setCurrent(null)} onRestore={(it) => setRestoreReq({ ids: [it.id], count: 1, size: it.size })} />}
            </div>
            <footer className="actionbar">
              <div className="summary">
                <b>{total.toLocaleString()}</b> files · {fmtSize(totalSize)}
                {selCount > 0 && <span className="selinfo"> · {selCount.toLocaleString()} selected</span>}
              </div>
              <button className="btn ghost" onClick={exportCsv} disabled={total === 0}>
                <IconExport /> Export list
              </button>
              <button className="btn ghost" onClick={() => setRestoreReq({ filter: dFilter, count: total, size: totalSize })} disabled={total === 0}>
                Restore all shown
              </button>
              <button className="btn primary" onClick={openRestore} disabled={selCount === 0}>
                <IconRestore /> Restore selected
              </button>
            </footer>
          </>
        )}
      </main>

      {restoreReq && (
        <RestoreDialog
          count={restoreReq.count}
          size={restoreReq.size}
          onClose={() => setRestoreReq(null)}
          onStart={(dest, keep) => api.startRestore({ ids: restoreReq.ids, filter: restoreReq.filter, dest, keepStructure: keep })}
        />
      )}
      {toast && (
        <div className="toast" onClick={() => setToast(null)}>
          {toast}
        </div>
      )}
    </div>
  );
}
