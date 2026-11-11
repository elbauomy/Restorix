import { fmtShort as fmtDate, fmtSize, METHOD_LABEL, METHOD_SHORT, SCORE_LABEL } from "../api";
import type { FileItem, Sort } from "../types";

interface Props {
  items: FileItem[];
  sort: Sort;
  setSort: (s: Sort) => void;
  selected: Set<number>;
  setSelected: (s: Set<number>) => void;
  allFiltered: boolean;
  setAllFiltered: (v: boolean) => void;
  current: number | null;
  onOpen: (it: FileItem) => void;
  onMore: () => void;
  hasMore: boolean;
  total: number;
}

const COLS: [string, string, string][] = [
  ["name", "Name", "col-name"],
  ["folder", "Original folder", "col-folder"],
  ["size", "Size", "col-size"],
  ["created", "Created", "col-date"],
  ["deleted", "Deleted", "col-date"],
  ["score", "Condition", "col-score"],
  ["method", "Found by", "col-method"],
];

export default function ResultsTable(p: Props) {
  const allOnPage = p.items.length > 0 && p.items.every((i) => p.selected.has(i.id));
  const toggleAll = () => {
    if (p.allFiltered || allOnPage) {
      p.setAllFiltered(false);
      p.setSelected(new Set());
    } else {
      p.setSelected(new Set(p.items.map((i) => i.id)));
    }
  };
  const toggle = (id: number) => {
    const s = new Set(p.selected);
    s.has(id) ? s.delete(id) : s.add(id);
    p.setAllFiltered(false);
    p.setSelected(s);
  };
  const sortBy = (field: string) => p.setSort({ field, desc: p.sort.field === field ? !p.sort.desc : field !== "name" && field !== "folder" });

  return (
    <div className="table-wrap">
      {allOnPage && !p.allFiltered && p.total > p.items.length && (
        <div className="select-all-bar">
          {p.items.length} files on screen are selected.{" "}
          <button className="btn link" onClick={() => p.setAllFiltered(true)}>
            Select all {p.total} matching files
          </button>
        </div>
      )}
      {p.allFiltered && (
        <div className="select-all-bar">
          All {p.total} matching files are selected.{" "}
          <button className="btn link" onClick={() => (p.setAllFiltered(false), p.setSelected(new Set()))}>
            Clear selection
          </button>
        </div>
      )}
      <table className="results">
        <thead>
          <tr>
            <th className="col-check">
              <input type="checkbox" checked={p.allFiltered || allOnPage} onChange={toggleAll} />
            </th>
            {COLS.map(([k, label, cls]) => (
              <th key={k} className={cls + " sortable"} onClick={() => sortBy(k)}>
                {label}
                {p.sort.field === k && <span className="sort-arrow">{p.sort.desc ? "▼" : "▲"}</span>}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {p.items.map((it) => (
            <tr key={it.id} className={(p.current === it.id ? "current " : "") + (p.allFiltered || p.selected.has(it.id) ? "sel" : "")} onClick={() => p.onOpen(it)}>
              <td className="col-check" onClick={(e) => e.stopPropagation()}>
                <input type="checkbox" checked={p.allFiltered || p.selected.has(it.id)} onChange={() => toggle(it.id)} />
              </td>
              <td className="col-name" title={it.name}>
                <div className="name-cell">
                  <span className={`ext-dot cat-${it.category}`}>{it.ext ? it.ext.slice(0, 4) : "?"}</span>
                  <span className="ellipsis">{it.name}</span>
                </div>
              </td>
              <td className="col-folder ellipsis" title={it.dir}>
                {it.dir}
              </td>
              <td className="col-size">{fmtSize(it.size)}</td>
              <td className="col-date">{fmtDate(it.created)}</td>
              <td className="col-date">{it.deleted ? fmtDate(it.deleted) : it.deleted_after ? <span className="muted" title="Deleted some time after this snapshot">&gt; {fmtDate(it.deleted_after)}</span> : <span className="muted">unknown</span>}</td>
              <td className="col-score">
                <span className={`pill score-${it.score}`}>{SCORE_LABEL[it.score]}</span>
              </td>
              <td className="col-method" title={METHOD_LABEL[it.method]}>{METHOD_SHORT[it.method]}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {p.hasMore && (
        <div className="more">
          <button className="btn ghost" onClick={p.onMore}>
            Show more ({p.items.length} of {p.total})
          </button>
        </div>
      )}
    </div>
  );
}
