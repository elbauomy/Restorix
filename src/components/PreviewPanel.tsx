import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { api, fmtDate, fmtSize, METHOD_LABEL, SCORE_LABEL } from "../api";
import type { FileItem, Preview } from "../types";
import { IconClose, IconRestore, IconWarn } from "./Icons";

interface Props {
  item: FileItem;
  onClose: () => void;
  onRestore: (it: FileItem) => void;
}

const SCORE_HELP: Record<string, string> = {
  good: "All of this file's data is still in free space. It should restore fully.",
  partial: "Some of this file's data has been reused or could not be located. Expect damage.",
  overwritten: "The space this file used now belongs to other data. Restoring is unlikely to give a usable file.",
  unknown: "Recoverability could not be determined.",
};

export default function PreviewPanel({ item, onClose, onRestore }: Props) {
  const [pv, setPv] = useState<Preview | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setPv(null);
    setErr(null);
    api
      .preview(item.id)
      .then((p) => alive && setPv(p))
      .catch((e) => alive && setErr(String(e)));
    return () => {
      alive = false;
    };
  }, [item.id]);

  const rows: [string, string][] = [
    ["Original folder", item.dir],
    ["Size", `${fmtSize(item.size)} (${item.size.toLocaleString()} bytes)`],
    ["Created", fmtDate(item.created) || "unknown"],
    ["Modified", fmtDate(item.modified) || "unknown"],
    ["Deleted", item.deleted ? fmtDate(item.deleted) : item.deleted_after ? `some time after ${fmtDate(item.deleted_after)}` : "unknown"],
    ["Found by", METHOD_LABEL[item.method]],
    ["File system", item.fs],
    ["Source", item.source],
  ];

  return (
    <aside className="preview">
      <div className="preview-head">
        <div className="preview-title" title={item.name}>
          {item.name}
        </div>
        <button className="icon-btn" onClick={onClose} title="Close">
          <IconClose />
        </button>
      </div>
      <div className="preview-body">
        <div className="preview-media">
          {!pv && !err && <div className="spinner" />}
          {err && <div className="muted small">{err}</div>}
          {pv?.kind === "image" && pv.dataUrl && <img src={pv.dataUrl} alt={item.name} />}
          {pv?.kind === "video" && pv.path && <video src={convertFileSrc(pv.path)} controls preload="metadata" />}
          {(pv?.kind === "text" || pv?.kind === "hex") && <pre className={pv.kind}>{pv.text}</pre>}
          {pv?.kind === "none" && <div className="muted small">No preview available</div>}
        </div>
        {pv?.note && (
          <div className="notice warn small">
            <IconWarn />
            <span>{pv.note}</span>
          </div>
        )}
        <div className={`score-card score-${item.score}`}>
          <span className={`pill score-${item.score}`}>{SCORE_LABEL[item.score]}</span>
          <span>{SCORE_HELP[item.score]}</span>
        </div>
        <dl className="meta">
          {rows.map(([k, v]) => (
            <div key={k}>
              <dt>{k}</dt>
              <dd>{v}</dd>
            </div>
          ))}
        </dl>
        {item.note && <div className="muted small">{item.note}</div>}
      </div>
      <div className="preview-foot">
        <button className="btn primary wide" onClick={() => onRestore(item)}>
          <IconRestore /> Restore this file…
        </button>
      </div>
    </aside>
  );
}
