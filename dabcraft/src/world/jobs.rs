// Job system: rayon gen/mesh tasks + crossbeam result channel. Binary consumer: Tasks 13/14.
#![cfg_attr(not(test), allow(dead_code))]

use crossbeam_channel::{Receiver, Sender};

use crate::mesh::greedy::Mesher;
use crate::mesh::neighborhood::MeshNeighborhood;
use crate::mesh::quad::PackedQuad;
use crate::world::chunks::{ColumnPos, SectionPos};
use crate::world::r#gen::{ColumnData, StructureWrite, WorldGen};

#[derive(Debug)]
pub enum JobResult {
    Generated { pos: ColumnPos, data: ColumnData, writes: Vec<StructureWrite> },
    Meshed { pos: SectionPos, quads: Vec<PackedQuad> },
}

/// Fire-and-forget rayon jobs with a crossbeam result channel (spec §3).
/// Priority comes from submission order: the caller submits nearest-first
/// each frame and caps in-flight counts, so the pool queue stays short and
/// camera-near work is never stuck behind a distant backlog.
pub struct Jobs {
    tx: Sender<JobResult>,
    rx: Receiver<JobResult>,
    pub gen_in_flight: usize,
    pub mesh_in_flight: usize,
}

impl Jobs {
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self { tx, rx, gen_in_flight: 0, mesh_in_flight: 0 }
    }

    pub fn spawn_gen(&mut self, worldgen: WorldGen, pos: ColumnPos) {
        self.gen_in_flight += 1;
        let tx = self.tx.clone();
        rayon::spawn(move || {
            let (data, writes) = worldgen.generate_column(pos.x, pos.z);
            // Send fails only when the app is shutting down; fine to drop.
            let _ = tx.send(JobResult::Generated { pos, data, writes });
        });
    }

    pub fn spawn_mesh(&mut self, pos: SectionPos, hood: MeshNeighborhood) {
        self.mesh_in_flight += 1;
        let tx = self.tx.clone();
        rayon::spawn(move || {
            let padded = hood.build_padded();
            let quads = Mesher::new().mesh(&padded);
            let _ = tx.send(JobResult::Meshed { pos, quads });
        });
    }

    /// Non-blocking: everything that finished since the last call.
    pub fn drain(&mut self) -> Vec<JobResult> {
        let mut out = Vec::new();
        while let Ok(r) = self.rx.try_recv() {
            match &r {
                JobResult::Generated { .. } => self.gen_in_flight -= 1,
                JobResult::Meshed { .. } => self.mesh_in_flight -= 1,
            }
            out.push(r);
        }
        out
    }
}

impl Default for Jobs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::chunks::ColumnPos;
    use crate::world::r#gen::WorldGen;

    #[test]
    fn gen_job_roundtrips_through_the_channel() {
        let mut jobs = Jobs::new();
        jobs.spawn_gen(WorldGen::new(7), ColumnPos { x: 0, z: 0 });
        assert_eq!(jobs.gen_in_flight, 1);
        // Worker pool latency: poll up to ~5 s.
        let mut results = Vec::new();
        for _ in 0..500 {
            results.extend(jobs.drain());
            if !results.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(results.len(), 1);
        assert_eq!(jobs.gen_in_flight, 0);
        match &results[0] {
            JobResult::Generated { pos, data, .. } => {
                assert_eq!(*pos, ColumnPos { x: 0, z: 0 });
                assert_eq!(data.sections.len(), 8);
            }
            other => panic!("expected Generated, got {other:?}"),
        }
    }
}
