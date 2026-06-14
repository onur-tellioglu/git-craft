use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Convert raw timestamp ticks (one begin/end pair per pass) to per-pass
/// milliseconds. An invalid pair (end <= begin: pass skipped this frame, or
/// Metal's occasional bad sample) yields None instead of a wrapped giant.
pub fn pass_millis(ticks: &[u64], period_ns: f32) -> Vec<Option<f32>> {
    ticks
        .chunks_exact(2)
        .map(|p| (p[1] > p[0]).then(|| (p[1] - p[0]) as f32 * period_ns / 1_000_000.0))
        .collect()
}

/// Measures N labeled GPU passes via pass-boundary timestamps (2 queries per
/// pass). Readback is async: `pass_ms` lags a few frames behind. While a
/// readback is pending, all `*_writes` return None and `resolve` is a no-op,
/// so a frame is either fully timed or not timed at all.
///
/// A pass that does not run in a given frame leaves its previous ticks in the
/// query set; the diff then repeats the old reading, which is the right HUD
/// behavior for cadenced passes (far shadow cascades).
pub struct GpuTimer {
    query_set: Option<wgpu::QuerySet>,
    resolve_buffer: wgpu::Buffer,
    read_buffer: wgpu::Buffer,
    map_done: Arc<AtomicBool>,
    map_ok: Arc<AtomicBool>,
    pending: bool,
    labels: &'static [&'static str],
    /// Per-pass milliseconds, indexed like `labels()`. Invalid samples keep
    /// the previous value.
    pub pass_ms: Vec<f32>,
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device, labels: &'static [&'static str]) -> Self {
        let enabled = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let count = (labels.len() * 2) as u32;
        let query_set = enabled.then(|| {
            device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count,
            })
        });
        let size = labels.len() as u64 * 16;
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts resolve"),
            size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts read"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            query_set,
            resolve_buffer,
            read_buffer,
            map_done: Arc::new(AtomicBool::new(false)),
            map_ok: Arc::new(AtomicBool::new(false)),
            pending: false,
            labels,
            pass_ms: vec![0.0; labels.len()],
        }
    }

    pub fn labels(&self) -> &'static [&'static str] {
        self.labels
    }

    pub fn total_ms(&self) -> f32 {
        self.pass_ms.iter().sum()
    }

    fn query_set_for(&self, pass: usize) -> Option<&wgpu::QuerySet> {
        debug_assert!(pass < self.labels.len());
        if self.pending {
            return None;
        }
        self.query_set.as_ref()
    }

    pub fn render_writes(&self, pass: usize) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.query_set_for(pass)
            .map(|qs| wgpu::RenderPassTimestampWrites {
                query_set: qs,
                beginning_of_pass_write_index: Some((pass * 2) as u32),
                end_of_pass_write_index: Some((pass * 2 + 1) as u32),
            })
    }

    /// Begin-only / end-only writes: one timing slot spanning a chain of
    /// passes (first pass gets begin, last pass gets end — used by bloom).
    pub fn render_writes_begin(&self, pass: usize) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.query_set_for(pass)
            .map(|qs| wgpu::RenderPassTimestampWrites {
                query_set: qs,
                beginning_of_pass_write_index: Some((pass * 2) as u32),
                end_of_pass_write_index: None,
            })
    }

    pub fn render_writes_end(&self, pass: usize) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.query_set_for(pass)
            .map(|qs| wgpu::RenderPassTimestampWrites {
                query_set: qs,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some((pass * 2 + 1) as u32),
            })
    }

    pub fn compute_writes(&self, pass: usize) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        self.query_set_for(pass)
            .map(|qs| wgpu::ComputePassTimestampWrites {
                query_set: qs,
                beginning_of_pass_write_index: Some((pass * 2) as u32),
                end_of_pass_write_index: Some((pass * 2 + 1) as u32),
            })
    }

    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.pending {
            return;
        }
        if let Some(qs) = &self.query_set {
            let n = (self.labels.len() * 2) as u32;
            encoder.resolve_query_set(qs, 0..n, &self.resolve_buffer, 0);
            encoder.copy_buffer_to_buffer(
                &self.resolve_buffer,
                0,
                &self.read_buffer,
                0,
                self.labels.len() as u64 * 16,
            );
        }
    }

    pub fn after_submit(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.query_set.is_none() {
            return;
        }
        if !self.pending {
            let done = self.map_done.clone();
            let ok = self.map_ok.clone();
            self.read_buffer
                .map_async(wgpu::MapMode::Read, .., move |result| {
                    // Signal completion even on failure, or `pending` locks forever
                    // and the timer silently stops measuring.
                    ok.store(result.is_ok(), Ordering::Release);
                    done.store(true, Ordering::Release);
                });
            self.pending = true;
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        if self.map_done.swap(false, Ordering::AcqRel) {
            if self.map_ok.load(Ordering::Acquire) {
                {
                    let data = self.read_buffer.get_mapped_range(..);
                    let ticks: &[u64] = bytemuck::cast_slice(&data);
                    let period = queue.get_timestamp_period();
                    for (i, ms) in pass_millis(ticks, period).into_iter().enumerate() {
                        if let Some(ms) = ms {
                            self.pass_ms[i] = ms;
                        }
                    }
                }
                self.read_buffer.unmap();
            } else {
                // Failed map leaves the buffer unmapped; just retry next frame.
                log::warn!("timestamp readback map failed; retrying");
            }
            self.pending = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_millis_converts_begin_end_pairs() {
        // Two passes: 1 ms and 3 ms at a 1 ns/tick period.
        let ms = pass_millis(&[0, 1_000_000, 2_000_000, 5_000_000], 1.0);
        assert_eq!(ms, vec![Some(1.0), Some(3.0)]);
    }

    #[test]
    fn invalid_samples_yield_none() {
        // end == begin (pass skipped / never written) and end < begin
        // (Metal glitch) must both be rejected, not wrap to huge values.
        let ms = pass_millis(&[5, 5, 9, 3], 1.0);
        assert_eq!(ms, vec![None, None]);
    }

    #[test]
    fn timestamp_period_scales_the_result() {
        let ms = pass_millis(&[0, 1000], 41.7);
        // 1000 ticks × 41.7 ns = 41.7 µs = 0.0417 ms
        assert!((ms[0].unwrap() - 0.0417).abs() < 1e-6);
    }
}
