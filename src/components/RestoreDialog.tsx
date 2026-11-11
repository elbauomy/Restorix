import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { useEffect, useState } from "react";
import { api, fmtSize } from "../api";
import type { RestoreProgress } from "../types";
import { IconClose, IconFolder, IconRestore, IconWarn } from "./Icons";

interface Props {
  count: number;
  size: number | null;
  onStart: (dest: string, keepStructure: boolean) => Promise<void>;
  onClose: () => void;
}

export default function RestoreDialog({ count, size, onStart, onClose }: Props) {
  const [dest, setDest] = useState(() => localStorage.getItem("restorix.dest") || "");
  const [keep, setKeep] = useState(() => localStorage.getItem("restorix.keep") !== "0");
  const [check, setCheck] = useState<{ sameDisk: boolean; message: string | null } | null>(null);
  const [prog, setProg] = useState<RestoreProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!dest) return setCheck(null);
    api.checkDestination(dest).then(setCheck).catch(() => setCheck(null));
  }, [dest]);

  useEffect(() => {
    if (!prog?.running) return;
    const t = setInterval(() => api.restoreProgress().then(setProg), 300);
    return () => clearInterval(t);
  }, [prog?.running]);

  async function pick() {
    const d = await open({ directory: true, multiple: false, title: "Choose where to save recovered files" });
    if (typeof d === "string") setDest(d);
  }

  async function go() {
    setError(null);
    try {
      localStorage.setItem("restorix.dest", dest);
      localStorage.setItem("restorix.keep", keep ? "1" : "0");
      await onStart(dest, keep);
      setProg(await api.restoreProgress());
    } catch (e) {
      setError(String(e));
    }
  }

  const done = prog && !prog.running;
  const pct = prog && prog.total ? (prog.done / prog.total) * 100 : 0;

  return (
    <div className="modal-back" onClick={() => !prog?.running && onClose()}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>
            <IconRestore /> Restore {count.toLocaleString()} file{count === 1 ? "" : "s"}
            {size != null && <span className="muted"> · {fmtSize(size)}</span>}
          </h2>
          {!prog?.running && (
            <button className="icon-btn" onClick={onClose}>
              <IconClose />
            </button>
          )}
        </div>

        {!prog && (
          <>
            <label className="field-label">Save recovered files to</label>
            <div className="dest-row">
              <input value={dest} readOnly placeholder="Choose a folder on a different drive…" onClick={pick} />
              <button className="btn ghost" onClick={pick}>
                <IconFolder /> Browse…
              </button>
            </div>
            {check?.sameDisk && (
              <div className="notice danger">
                <IconWarn />
                <span>{check.message}</span>
              </div>
            )}
            <label className="check">
              <input type="checkbox" checked={keep} onChange={(e) => setKeep(e.target.checked)} />
              <span>
                <b>Keep the original folder structure</b>
                <small>Recreates the folders the files came from, inside the destination</small>
              </span>
            </label>
            <div className="muted small">The drive being scanned is only read, never written. A CSV report of everything restored is saved with the files.</div>
            {error && (
              <div className="notice danger">
                <IconWarn />
                <span>{error}</span>
              </div>
            )}
            <div className="modal-foot">
              <button className="btn ghost" onClick={onClose}>
                Cancel
              </button>
              <button className="btn primary" disabled={!dest || check?.sameDisk} onClick={go}>
                <IconRestore /> Restore
              </button>
            </div>
          </>
        )}

        {prog && (
          <>
            <div className="bar big">
              <div className="bar-fill" style={{ width: `${pct}%` }} />
            </div>
            <div className="scan-stats">
              <span>
                {prog.done} / {prog.total} files · {fmtSize(prog.bytes)}
              </span>
              <span>{prog.failed > 0 ? `${prog.failed} failed` : ""}</span>
            </div>
            {done && (
              <div className={"notice " + (prog.failed ? "warn" : "ok")}>
                <span>
                  Restored {prog.ok} file{prog.ok === 1 ? "" : "s"}
                  {prog.failed ? `, ${prog.failed} could not be restored` : ""}.
                </span>
              </div>
            )}
            {prog.errors.length > 0 && (
              <pre className="errors">{prog.errors.slice(0, 50).join("\n")}</pre>
            )}
            <div className="modal-foot">
              {prog.running ? (
                <button className="btn danger" onClick={() => api.cancelRestore()}>
                  Stop
                </button>
              ) : (
                <>
                  <button className="btn ghost" onClick={() => openPath(prog.dest)}>
                    <IconFolder /> Open folder
                  </button>
                  <button className="btn primary" onClick={onClose}>
                    Done
                  </button>
                </>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
