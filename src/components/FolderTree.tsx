import { useMemo, useState } from "react";
import type { FolderNode } from "../types";
import { IconChevron, IconFolder } from "./Icons";

interface Props {
  nodes: FolderNode[];
  selected: string | null;
  onSelect: (path: string | null) => void;
  total: number;
}

export default function FolderTree({ nodes, selected, onSelect, total }: Props) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const hasChildren = useMemo(() => {
    const s = new Set<string>();
    for (const n of nodes) {
      const parent = n.path.slice(0, n.path.lastIndexOf("/"));
      if (parent) s.add(parent);
    }
    return s;
  }, [nodes]);

  const visible = nodes.filter((n) => {
    // hidden if any ancestor is collapsed
    let p = n.path;
    while (true) {
      const i = p.lastIndexOf("/");
      if (i <= 0) return true;
      p = p.slice(0, i);
      if (collapsed.has(p)) return false;
    }
  });

  const toggle = (path: string) => {
    const s = new Set(collapsed);
    s.has(path) ? s.delete(path) : s.add(path);
    setCollapsed(s);
  };

  return (
    <div className="tree">
      <div className="tree-head">Folders</div>
      <button className={"tree-row" + (selected == null ? " active" : "")} onClick={() => onSelect(null)}>
        <span className="tree-twisty" />
        <IconFolder open />
        <span className="tree-name">All results</span>
        <span className="count">{total}</span>
      </button>
      {visible.map((n) => (
        <div key={n.path} className={"tree-row" + (selected === n.path ? " active" : "")} style={{ paddingLeft: 8 + n.depth * 14 }} onClick={() => onSelect(n.path)} title={n.path}>
          <span
            className="tree-twisty"
            onClick={(e) => {
              e.stopPropagation();
              if (hasChildren.has(n.path)) toggle(n.path);
            }}
          >
            {hasChildren.has(n.path) && <IconChevron open={!collapsed.has(n.path)} />}
          </span>
          <IconFolder open={!collapsed.has(n.path) && hasChildren.has(n.path)} />
          <span className="tree-name">{n.name.replace("∕", "/")}</span>
          <span className="count">{n.count}</span>
        </div>
      ))}
      {nodes.length === 0 && <div className="muted small pad">No folders yet</div>}
    </div>
  );
}
