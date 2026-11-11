//! Search, filter, sort and folder-tree over scan results.

use crate::model::*;
use crate::util::glob_match;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Filter {
    /// Name search; supports `*` and `?` wildcards, otherwise "contains".
    pub text: String,
    pub categories: Vec<Category>,
    pub exts: Vec<String>,
    pub methods: Vec<Method>,
    pub scores: Vec<Score>,
    pub deleted_from: Option<i64>,
    pub deleted_to: Option<i64>,
    pub created_from: Option<i64>,
    pub created_to: Option<i64>,
    /// Keep items whose date is unknown when a date range is set.
    pub include_unknown_dates: bool,
    pub size_min: Option<u64>,
    pub size_max: Option<u64>,
    /// Folder selected in the tree (matches the folder and its sub-folders).
    pub folder: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Sort {
    pub field: String,
    pub desc: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub total: usize,
    pub total_size: u64,
    pub items: Vec<FileItem>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FolderNode {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub count: usize,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub total: usize,
    pub by_category: BTreeMap<String, usize>,
    pub by_method: BTreeMap<String, usize>,
    pub by_score: BTreeMap<String, usize>,
    pub exts: Vec<(String, usize)>,
}

struct Compiled {
    f: Filter,
    pat: Option<Vec<char>>,
    text: String,
    exts: Vec<String>,
}

fn in_range(v: Option<i64>, from: Option<i64>, to: Option<i64>, unknown_ok: bool) -> bool {
    if from.is_none() && to.is_none() {
        return true;
    }
    match v {
        None => unknown_ok,
        Some(t) => from.map_or(true, |f| t >= f) && to.map_or(true, |e| t <= e),
    }
}

impl Compiled {
    fn new(f: &Filter) -> Self {
        let text = f.text.trim().to_lowercase();
        let pat = if text.contains('*') || text.contains('?') { Some(text.chars().collect()) } else { None };
        let exts = f.exts.iter().map(|e| e.trim_start_matches('.').to_lowercase()).collect();
        Compiled { f: f.clone(), pat, text, exts }
    }

    fn matches(&self, it: &FileItem, ignore_folder: bool) -> bool {
        let f = &self.f;
        if !self.text.is_empty() {
            let name = it.name.to_lowercase();
            let ok = match &self.pat {
                Some(p) => glob_match(p, &name.chars().collect::<Vec<_>>()),
                None => name.contains(&self.text) || it.dir.to_lowercase().contains(&self.text),
            };
            if !ok {
                return false;
            }
        }
        if !f.categories.is_empty() && !f.categories.contains(&it.category) {
            return false;
        }
        if !self.exts.is_empty() && !self.exts.contains(&it.ext) {
            return false;
        }
        if !f.methods.is_empty() && !f.methods.contains(&it.method) {
            return false;
        }
        if !f.scores.is_empty() && !f.scores.contains(&it.score) {
            return false;
        }
        let del = it.deleted.or(it.deleted_after);
        if !in_range(del, f.deleted_from, f.deleted_to, f.include_unknown_dates) {
            return false;
        }
        if !in_range(it.created, f.created_from, f.created_to, f.include_unknown_dates) {
            return false;
        }
        if f.size_min.is_some_and(|m| it.size < m) || f.size_max.is_some_and(|m| it.size > m) {
            return false;
        }
        if !ignore_folder {
            if let Some(folder) = &f.folder {
                let p = folder_key(it);
                if !(p == *folder || p.starts_with(&format!("{folder}/")) || folder == "/") {
                    return false;
                }
            }
        }
        true
    }
}

/// Tree key for an item: source label + original folder.
pub fn folder_key(it: &FileItem) -> String {
    let src = it.source.replace('/', "∕");
    if it.dir == "/" || it.dir.is_empty() {
        format!("/{src}")
    } else {
        format!("/{src}{}", it.dir)
    }
}

pub fn select<'a>(items: &'a [FileItem], f: &Filter) -> Vec<&'a FileItem> {
    let c = Compiled::new(f);
    items.iter().filter(|it| c.matches(it, false)).collect()
}

pub fn page(items: &[FileItem], f: &Filter, sort: &Sort, offset: usize, limit: usize) -> Page {
    let mut v = select(items, f);
    let key_i = |o: Option<i64>| o.unwrap_or(i64::MIN);
    match sort.field.as_str() {
        "name" => v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        "folder" => v.sort_by(|a, b| a.dir.cmp(&b.dir).then(a.name.cmp(&b.name))),
        "size" => v.sort_by_key(|i| i.size),
        "created" => v.sort_by_key(|i| key_i(i.created)),
        "modified" => v.sort_by_key(|i| key_i(i.modified)),
        "deleted" => v.sort_by_key(|i| key_i(i.deleted.or(i.deleted_after))),
        "type" => v.sort_by(|a, b| a.ext.cmp(&b.ext)),
        "score" => v.sort_by_key(|i| i.score),
        "method" => v.sort_by_key(|i| i.method),
        _ => {}
    }
    if sort.desc {
        v.reverse();
    }
    let total = v.len();
    let total_size = v.iter().map(|i| i.size).sum();
    let items = v.into_iter().skip(offset).take(limit).cloned().collect();
    Page { total, total_size, items }
}

/// Folder tree of items matching the filter (ignoring the folder selection itself).
pub fn tree(items: &[FileItem], f: &Filter) -> Vec<FolderNode> {
    let c = Compiled::new(f);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for it in items.iter().filter(|it| c.matches(it, true)) {
        let key = folder_key(it);
        let mut acc = String::new();
        for part in key.split('/').filter(|p| !p.is_empty()) {
            acc.push('/');
            acc.push_str(part);
            *counts.entry(acc.clone()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .map(|(path, count)| {
            let depth = path.matches('/').count() - 1;
            let name = path.rsplit('/').next().unwrap_or("").to_string();
            FolderNode { path, name, depth, count }
        })
        .collect()
}

pub fn stats(items: &[FileItem]) -> Stats {
    let mut s = Stats { total: items.len(), ..Default::default() };
    let mut exts: BTreeMap<String, usize> = BTreeMap::new();
    let name = |v: &dyn erased::Ser| v.to();
    for it in items {
        *s.by_category.entry(name(&it.category)).or_default() += 1;
        *s.by_method.entry(name(&it.method)).or_default() += 1;
        *s.by_score.entry(name(&it.score)).or_default() += 1;
        if !it.ext.is_empty() {
            *exts.entry(it.ext.clone()).or_default() += 1;
        }
    }
    let mut ev: Vec<(String, usize)> = exts.into_iter().collect();
    ev.sort_by(|a, b| b.1.cmp(&a.1));
    ev.truncate(40);
    s.exts = ev;
    s
}

mod erased {
    pub trait Ser {
        fn to(&self) -> String;
    }
    impl<T: serde::Serialize> Ser for T {
        fn to(&self) -> String {
            serde_json::to_value(self).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
        }
    }
}
