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

/// A finished load or save acknowledgement, drained by the main loop.
pub enum Loaded {
    Column {
        pos: ColumnPos,
        data: ColumnData,
        light: Box<[LightData; COLUMN_SECTIONS]>,
    },
    /// Disk error or corrupt payload — the caller regenerates instead.
    Failed { pos: ColumnPos },
    /// The worker successfully wrote the column to disk. The caller may now
    /// move the column into `saved_columns`.
    SaveOk { pos: ColumnPos },
    /// The worker failed to write the column (disk full, permission error, …).
    /// The caller should log a visible error and not mark the column as saved.
    SaveFailed { pos: ColumnPos },
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

    /// Non-blocking: every finished load or save acknowledgement since the
    /// last call. Only load results count against `load_in_flight`; save acks
    /// (`SaveOk` / `SaveFailed`) do not.
    pub fn drain_loaded(&mut self) -> Vec<Loaded> {
        let mut out = Vec::new();
        while let Ok(r) = self.rx.try_recv() {
            match &r {
                Loaded::Column { .. } | Loaded::Failed { .. } => {
                    self.load_in_flight = self.load_in_flight.saturating_sub(1);
                }
                Loaded::SaveOk { .. } | Loaded::SaveFailed { .. } => {}
            }
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
                let ack = match store.save_column(pos, serialize_column(&sections)) {
                    Ok(()) => Loaded::SaveOk { pos },
                    Err(e) => {
                        log::error!("failed to save column {pos:?}: {e}");
                        Loaded::SaveFailed { pos }
                    }
                };
                let _ = res_tx.send(ack);
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

    /// Block on the worker for one finished *load* result (~5 s budget).
    /// Save acknowledgements (SaveOk / SaveFailed) are drained but skipped
    /// so tests that mix saves and loads get the load result they expect.
    fn poll_one(p: &mut Persistence) -> Loaded {
        for _ in 0..500 {
            for item in p.drain_loaded() {
                match item {
                    Loaded::SaveOk { .. } | Loaded::SaveFailed { .. } => {
                        // skip save acks; keep polling for the load result
                    }
                    other => return other,
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("worker produced no load result within the time budget");
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
            // poll_one skips save acks; these arms are unreachable in practice.
            Loaded::SaveOk { .. } | Loaded::SaveFailed { .. } => {
                unreachable!("poll_one filters out save acks")
            }
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

    /// Write a region file with a valid GCR1 header but a corrupt section
    /// payload (palette_len=1, bits=2, packed indices all out of range) and
    /// assert the worker returns Loaded::Failed without panicking.
    ///
    /// This tests the full path — from on-disk bytes through region parsing,
    /// deserialization, and light recomputation — where finding #1 would have
    /// panicked the worker before the bounds check was added.
    #[test]
    fn loading_a_corrupt_region_file_returns_failed() {
        use crate::world::region::{local_index, region_of, serialize_region};
        use std::collections::BTreeMap;

        let dir = temp_dir("corrupt");
        let pos = ColumnPos { x: 7, z: -3 };

        // Build a corrupt section payload: palette_len=1, bits=2,
        // all data words 0xFF (every 2-bit index == 3, out of range for palette_len=1).
        const VOLUME: usize = 32 * 32 * 32;
        let words = VOLUME * 2 / 64; // 1024 words
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u16.to_le_bytes()); // palette_len = 1
        payload.extend_from_slice(&0u16.to_le_bytes()); // palette[0] = AIR (BlockId 0)
        payload.push(2u8); // bits = 2
        for _ in 0..words {
            payload.extend_from_slice(&u64::MAX.to_le_bytes());
        }
        // Repeat for all COLUMN_SECTIONS so the region payload looks like a full column.
        use crate::world::r#gen::COLUMN_SECTIONS;
        let full_payload: Vec<u8> = payload.repeat(COLUMN_SECTIONS);

        // Wrap in a valid region file structure.
        let (rx, rz) = region_of(pos);
        let idx = local_index(pos);
        let mut map = BTreeMap::new();
        map.insert(idx, full_payload);
        let region_blob = serialize_region(&map);

        // Write the region file to disk in the store dir.
        std::fs::create_dir_all(&dir).unwrap();
        let region_path = dir.join(format!("r.{rx}.{rz}.gcr"));
        std::fs::write(&region_path, &region_blob).unwrap();

        // Open persistence pointing at this dir; it will scan and find pos as saved.
        let (mut p, saved) = Persistence::new(dir);
        assert!(saved.contains(&pos), "scan must find the corrupt column");

        // Request a load; the worker must return Failed, not panic.
        p.request_load(pos);
        assert!(
            matches!(poll_one(&mut p), Loaded::Failed { .. }),
            "corrupt payload must return Loaded::Failed, not panic"
        );
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
            // poll_one skips save acks; these arms are unreachable in practice.
            Loaded::SaveOk { .. } | Loaded::SaveFailed { .. } => {
                unreachable!("poll_one filters out save acks")
            }
        }
        p.shutdown();
    }
}
