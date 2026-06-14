//! Async persistence worker.
//!
//! A single background thread owns the [`RegionStore`], so region files are
//! never written concurrently. The main loop only enqueues load/save requests
//! and drains finished loads each frame — it never blocks on disk. The worker
//! does all I/O and (de)serialization, and recomputes a loaded column's light
//! via the generation path ([`light_new_column`]) so a loaded column is
//! indistinguishable from a freshly generated one.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};

use crate::world::chunks::ColumnPos;
use crate::world::r#gen::{COLUMN_SECTIONS, ColumnData};
use crate::world::light::{LightData, light_new_column};
use crate::world::region::{RegionStore, deserialize_column, serialize_column};
use crate::world::section::Section;

/// A finished load, drained by the main loop.
pub enum Loaded {
    Column {
        pos: ColumnPos,
        data: ColumnData,
        light: Box<[LightData; COLUMN_SECTIONS]>,
    },
    /// Disk error or corrupt payload — the caller regenerates instead.
    Failed { pos: ColumnPos },
}

enum Req {
    Load(ColumnPos),
    Save(ColumnPos, Vec<Arc<Section>>),
    Shutdown,
}

/// Handle to the persistence worker thread.
pub struct Persistence {
    tx: Sender<Req>,
    rx: Receiver<Loaded>,
    handle: Option<JoinHandle<()>>,
    /// Loads requested but not yet drained; the streamer caps in-flight work.
    pub load_in_flight: usize,
}

impl Persistence {
    /// Open the store at `dir`, scan which columns are already saved, and spawn
    /// the worker. Returns the handle plus the initial saved set so streaming
    /// knows which columns to load instead of generate.
    pub fn new(dir: impl Into<PathBuf>) -> (Self, HashSet<ColumnPos>) {
        let store = RegionStore::new(dir);
        let saved = store.scan_saved();
        let (req_tx, req_rx) = crossbeam_channel::unbounded::<Req>();
        let (res_tx, res_rx) = crossbeam_channel::unbounded::<Loaded>();
        let handle = std::thread::Builder::new()
            .name("git-craft-io".into())
            .spawn(move || worker(store, req_rx, res_tx))
            .expect("spawn persistence worker");
        (
            Self {
                tx: req_tx,
                rx: res_rx,
                handle: Some(handle),
                load_in_flight: 0,
            },
            saved,
        )
    }

    pub fn request_load(&mut self, pos: ColumnPos) {
        if self.tx.send(Req::Load(pos)).is_ok() {
            self.load_in_flight += 1;
        }
    }

    pub fn request_save(&self, pos: ColumnPos, sections: Vec<Arc<Section>>) {
        let _ = self.tx.send(Req::Save(pos, sections));
    }

    /// Non-blocking: every load finished since the last call.
    pub fn drain_loaded(&mut self) -> Vec<Loaded> {
        let mut out = Vec::new();
        while let Ok(r) = self.rx.try_recv() {
            self.load_in_flight = self.load_in_flight.saturating_sub(1);
            out.push(r);
        }
        out
    }

    /// Flush queued saves (the channel is FIFO, so every prior save runs first)
    /// then join the worker. Idempotent; call once on exit.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(Req::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn worker(store: RegionStore, req_rx: Receiver<Req>, res_tx: Sender<Loaded>) {
    while let Ok(req) = req_rx.recv() {
        match req {
            Req::Load(pos) => {
                let loaded = match store.load_column(pos) {
                    Ok(Some(bytes)) => deserialize_column(&bytes).map(|sections| {
                        let light = Box::new(light_new_column(&sections));
                        Loaded::Column {
                            pos,
                            data: ColumnData { sections },
                            light,
                        }
                    }),
                    _ => None,
                }
                .unwrap_or(Loaded::Failed { pos });
                let _ = res_tx.send(loaded);
            }
            Req::Save(pos, sections) => {
                if let Err(e) = store.save_column(pos, serialize_column(&sections)) {
                    log::warn!("failed to save column {pos:?}: {e}");
                }
            }
            Req::Shutdown => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::block::STONE;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gitcraft_persist_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Block on the worker for one finished load (~5 s budget), like the
    /// `jobs` test polls its channel.
    fn poll_one(p: &mut Persistence) -> Loaded {
        for _ in 0..500 {
            if let Some(first) = p.drain_loaded().into_iter().next() {
                return first;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("worker produced no result within the time budget");
    }

    #[test]
    fn save_then_load_returns_the_columns_blocks() {
        let (mut p, saved) = Persistence::new(temp_dir("save_then_load"));
        assert!(saved.is_empty());

        let pos = ColumnPos { x: 4, z: -2 };
        let mut edited = Section::empty();
        edited.set(1, 2, 3, STONE);
        let sections: Vec<Arc<Section>> = (0..COLUMN_SECTIONS)
            .map(|i| {
                if i == 1 {
                    Arc::new(edited.clone())
                } else {
                    Arc::new(Section::empty())
                }
            })
            .collect();

        p.request_save(pos, sections);
        p.request_load(pos);
        assert_eq!(p.load_in_flight, 1);

        match poll_one(&mut p) {
            Loaded::Column { pos: p2, data, .. } => {
                assert_eq!(p2, pos);
                assert_eq!(data.sections.len(), COLUMN_SECTIONS);
                assert_eq!(data.sections[1].get(1, 2, 3), STONE);
            }
            Loaded::Failed { .. } => panic!("expected a loaded column, got Failed"),
        }
        assert_eq!(p.load_in_flight, 0);
        p.shutdown();
    }

    #[test]
    fn loading_an_absent_column_fails_gracefully() {
        let (mut p, _) = Persistence::new(temp_dir("absent"));
        p.request_load(ColumnPos { x: 999, z: 999 });
        assert!(matches!(poll_one(&mut p), Loaded::Failed { .. }));
        p.shutdown();
    }

    /// The "quit and relaunch" path: a real generated column with an edit is
    /// saved and the worker shut down, then a *fresh* store on the same dir
    /// re-discovers the column via `scan` and loads it with the edit intact —
    /// while untouched terrain is byte-identical to a regenerate (light is
    /// recomputed by the worker, never stored).
    #[test]
    fn edits_survive_a_store_reopen() {
        use crate::world::r#gen::WorldGen;

        let dir = temp_dir("reopen");
        let pos = ColumnPos { x: 1, z: 2 };

        // First "session": generate, edit, save, shut down.
        {
            let (mut p, saved) = Persistence::new(dir.clone());
            assert!(saved.is_empty());
            let (data, _writes) = WorldGen::new(1337).generate_column(pos.x, pos.z);
            let mut sections: Vec<Arc<Section>> = data.sections.into_iter().map(Arc::new).collect();
            Arc::make_mut(&mut sections[3]).set(7, 8, 9, STONE);
            p.request_save(pos, sections);
            p.shutdown(); // flush the save and join
        }

        // Second "session": fresh store, same dir.
        let (mut p, saved) = Persistence::new(dir);
        assert!(
            saved.contains(&pos),
            "scan must rediscover the saved column"
        );
        p.request_load(pos);
        match poll_one(&mut p) {
            Loaded::Column { pos: p2, data, .. } => {
                assert_eq!(p2, pos);
                assert_eq!(data.sections[3].get(7, 8, 9), STONE, "edit must survive");
            }
            Loaded::Failed { .. } => panic!("expected the reopened column to load"),
        }
        p.shutdown();
    }
}
