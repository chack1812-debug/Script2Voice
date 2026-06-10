use crate::scene_line::LabParams;
use std::collections::VecDeque;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: usize,
    pub params: LabParams,
    pub wav: PathBuf,
}

pub struct History {
    entries: VecDeque<HistoryEntry>,
    next_id: usize,
    cap: usize,
    pub sel_a: Option<usize>,
    pub sel_b: Option<usize>,
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            next_id: 1,
            cap,
            sel_a: None,
            sel_b: None,
        }
    }

    /// 追加して採番した id を返す。あふれた分は WAV ファイルも削除する。
    pub fn push(&mut self, params: LabParams, wav: PathBuf) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push_back(HistoryEntry { id, params, wav });
        while self.entries.len() > self.cap {
            if let Some(old) = self.entries.pop_front() {
                let _ = std::fs::remove_file(&old.wav);
                if self.sel_a == Some(old.id) {
                    self.sel_a = None;
                }
                if self.sel_b == Some(old.id) {
                    self.sel_b = None;
                }
            }
        }
        id
    }

    pub fn entries(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.entries.iter()
    }

    pub fn get(&self, id: usize) -> Option<&HistoryEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// A→B の順に選択。選択済み id の再指定は解除。両方埋まっていたら B を置換。
    pub fn toggle_select(&mut self, id: usize) {
        if self.sel_a == Some(id) {
            self.sel_a = None;
            return;
        }
        if self.sel_b == Some(id) {
            self.sel_b = None;
            return;
        }
        if self.sel_a.is_none() {
            self.sel_a = Some(id);
            return;
        }
        self.sel_b = Some(id);
    }

    pub fn clear(&mut self) {
        for e in self.entries.drain(..) {
            let _ = std::fs::remove_file(&e.wav);
        }
        self.sel_a = None;
        self.sel_b = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(cap: usize) -> History {
        History::new(cap)
    }

    #[test]
    fn push_assigns_sequential_ids_and_caps_fifo() {
        let mut hist = h(3);
        for i in 0..5 {
            hist.push(LabParams::default(), PathBuf::from(format!("{i}.wav")));
        }
        let ids: Vec<usize> = hist.entries().map(|e| e.id).collect();
        assert_eq!(ids, vec![3, 4, 5], "古い順に追い出し・id は1始まり連番");
    }

    #[test]
    fn eviction_clears_dangling_selection() {
        let mut hist = h(2);
        let first = hist.push(LabParams::default(), "a.wav".into());
        hist.toggle_select(first);
        assert_eq!(hist.sel_a, Some(first));
        hist.push(LabParams::default(), "b.wav".into());
        hist.push(LabParams::default(), "c.wav".into()); // first が追い出される
        assert_eq!(hist.sel_a, None);
    }

    #[test]
    fn toggle_select_fills_a_then_b_then_replaces_b() {
        let mut hist = h(10);
        let a = hist.push(LabParams::default(), "a.wav".into());
        let b = hist.push(LabParams::default(), "b.wav".into());
        let c = hist.push(LabParams::default(), "c.wav".into());
        hist.toggle_select(a);
        hist.toggle_select(b);
        assert_eq!((hist.sel_a, hist.sel_b), (Some(a), Some(b)));
        hist.toggle_select(c); // 両方埋まり → B を置換
        assert_eq!((hist.sel_a, hist.sel_b), (Some(a), Some(c)));
        hist.toggle_select(a); // 再クリックで解除
        assert_eq!(hist.sel_a, None);
    }
}
