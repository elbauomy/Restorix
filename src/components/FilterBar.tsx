import { useState } from "react";
import { CATEGORY_LABEL, dayToUnix, METHOD_LABEL, SCORE_LABEL, unixToDay } from "../api";
import { emptyFilter, type Category, type Filter, type Method, type Score, type Stats } from "../types";
import { IconClose, IconFilter, IconSearch } from "./Icons";

interface Props {
  filter: Filter;
  setFilter: (f: Filter) => void;
  stats: Stats | null;
}

function toggle<T>(arr: T[], v: T): T[] {
  return arr.includes(v) ? arr.filter((x) => x !== v) : [...arr, v];
}

export default function FilterBar({ filter, setFilter, stats }: Props) {
  const [more, setMore] = useState(false);
  const f = filter;
  const set = (patch: Partial<Filter>) => setFilter({ ...f, ...patch });
  const count = (rec: Record<string, number> | undefined, k: string) => rec?.[k] ?? 0;
  const active =
    f.categories.length + f.methods.length + f.scores.length + f.exts.length + [f.deletedFrom, f.deletedTo, f.createdFrom, f.createdTo, f.sizeMin, f.sizeMax].filter((x) => x != null).length;

  return (
    <div className="filterbar">
      <div className="filter-row">
        <div className="search">
          <IconSearch />
          <input value={f.text} placeholder="Search by name (wildcards: *.jpg, IMG_20??*)" onChange={(e) => set({ text: e.target.value })} />
          {f.text && (
            <button className="icon-btn" onClick={() => set({ text: "" })} title="Clear">
              <IconClose />
            </button>
          )}
        </div>
        <div className="seg">
          {(Object.keys(CATEGORY_LABEL) as Category[]).map((c) => (
            <button key={c} className={"seg-btn" + (f.categories.includes(c) ? " on" : "")} onClick={() => set({ categories: toggle(f.categories, c) })}>
              {CATEGORY_LABEL[c]}
              <span className="count">{count(stats?.byCategory, c)}</span>
            </button>
          ))}
        </div>
        <button className={"btn ghost" + (more ? " on" : "")} onClick={() => setMore(!more)}>
          <IconFilter /> Filters{active > 0 && <span className="badge">{active}</span>}
        </button>
        {(active > 0 || f.text) && (
          <button className="btn link" onClick={() => setFilter({ ...emptyFilter, folder: f.folder })}>
            Reset
          </button>
        )}
      </div>

      {more && (
        <div className="filter-more">
          <div className="fgroup">
            <label>Deleted between</label>
            <div className="range">
              <input type="date" value={unixToDay(f.deletedFrom)} onChange={(e) => set({ deletedFrom: dayToUnix(e.target.value, false) })} />
              <span>and</span>
              <input type="date" value={unixToDay(f.deletedTo)} onChange={(e) => set({ deletedTo: dayToUnix(e.target.value, true) })} />
            </div>
          </div>
          <div className="fgroup">
            <label>Created between</label>
            <div className="range">
              <input type="date" value={unixToDay(f.createdFrom)} onChange={(e) => set({ createdFrom: dayToUnix(e.target.value, false) })} />
              <span>and</span>
              <input type="date" value={unixToDay(f.createdTo)} onChange={(e) => set({ createdTo: dayToUnix(e.target.value, true) })} />
            </div>
            <label className="check slim">
              <input type="checkbox" checked={f.includeUnknownDates} onChange={(e) => set({ includeUnknownDates: e.target.checked })} />
              <span>Include files whose date is unknown</span>
            </label>
          </div>
          <div className="fgroup">
            <label>Size (MB)</label>
            <div className="range">
              <input type="number" min={0} placeholder="min" value={f.sizeMin != null ? f.sizeMin / 1048576 : ""} onChange={(e) => set({ sizeMin: e.target.value === "" ? null : Math.round(Number(e.target.value) * 1048576) })} />
              <span>to</span>
              <input type="number" min={0} placeholder="max" value={f.sizeMax != null ? f.sizeMax / 1048576 : ""} onChange={(e) => set({ sizeMax: e.target.value === "" ? null : Math.round(Number(e.target.value) * 1048576) })} />
            </div>
          </div>
          <div className="fgroup">
            <label>Found by</label>
            <div className="chips">
              {(Object.keys(METHOD_LABEL) as Method[]).map((m) => (
                <button key={m} className={"chip" + (f.methods.includes(m) ? " on" : "")} onClick={() => set({ methods: toggle(f.methods, m) })}>
                  {METHOD_LABEL[m]} <span className="count">{count(stats?.byMethod, m)}</span>
                </button>
              ))}
            </div>
          </div>
          <div className="fgroup">
            <label>Recoverability</label>
            <div className="chips">
              {(Object.keys(SCORE_LABEL) as Score[]).map((s) => (
                <button key={s} className={`chip score-${s}` + (f.scores.includes(s) ? " on" : "")} onClick={() => set({ scores: toggle(f.scores, s) })}>
                  {SCORE_LABEL[s]} <span className="count">{count(stats?.byScore, s)}</span>
                </button>
              ))}
            </div>
          </div>
          {stats && stats.exts.length > 0 && (
            <div className="fgroup wide">
              <label>File type</label>
              <div className="chips">
                {stats.exts.map(([e, n]) => (
                  <button key={e} className={"chip" + (f.exts.includes(e) ? " on" : "")} onClick={() => set({ exts: toggle(f.exts, e) })}>
                    .{e} <span className="count">{n}</span>
                  </button>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
