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

/// Compute the true whole-frame GPU wall-clock from the raw timestamp buffer.
/// Returns `(max_end_tick − min_begin_tick) × period_ns / 1_000_000` ms.
///
/// On Apple TBDR, consecutive passes pipeline (pass N fragment overlaps pass
/// N+1 tiler), so summing per-pass deltas overcounts. The wall-clock is the
/// only accurate total.
///
/// Zero ticks are treated as "pass did not run" and are excluded from the
/// min/max reduction. Returns `None` if no valid (non-zero begin, non-zero end,
/// end > begin) pair exists.
pub fn frame_wall_ms(ticks: &[u64], period_ns: f32) -> Option<f32> {
    let mut min_begin = u64::MAX;
    let mut max_end = 0u64;
    for pair in ticks.chunks_exact(2) {
        let (begin, end) = (pair[0], pair[1]);
        // Skip pairs where either tick is zero (pass was not issued this frame)
        // or where end ≤ begin (Metal occasional bad sample).
        if begin == 0 || end == 0 || end <= begin {
            continue;
        }
        if begin < min_begin {
            min_begin = begin;
        }
        if end > max_end {
            max_end = end;
        }
    }
    // max_end == 0  ⟺  no valid pair found (also implies min_begin == u64::MAX).
    // When at least one valid pair exists, max_end > min_begin is guaranteed
    // by the end > begin filter above.
    if max_end == 0 {
        return None;
    }
    debug_assert!(
        min_begin < max_end,
        "invariant: valid pair guarantees max_end > min_begin"
    );
    Some((max_end - min_begin) as f32 * period_ns / 1_000_000.0)
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
    /// True whole-frame GPU wall-clock (see `frame_wall_ms`). Updated each
    /// time `after_submit` completes a successful readback. Zero before the
    /// first readback.
    pub wall_ms: f32,
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
            wall_ms: 0.0,
        }
    }

    pub fn labels(&self) -> &'static [&'static str] {
        self.labels
    }

    /// Returns true when TIMESTAMP_QUERY is available and the query set was
    /// successfully created. This is the canonical ground truth for whether GPU
    /// timestamps are enabled — prefer it over heuristics like `total_ms() > 0`.
    pub fn timestamps_enabled(&self) -> bool {
        self.query_set.is_some()
    }

    /// True whole-frame GPU wall-clock in ms. Derived from the raw timestamp
    /// buffer on each successful readback. Zero before the first readback.
    ///
    /// On Apple TBDR, consecutive render passes pipeline; summing per-pass deltas
    /// overcounts by ~3×. This is the accurate replacement.
    pub fn total_ms(&self) -> f32 {
        self.wall_ms
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
                    if let Some(w) = frame_wall_ms(ticks, period) {
                        self.wall_ms = w;
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

    #[test]
    fn frame_wall_ms_uses_first_begin_and_last_end() {
        // Two passes at 1 ns/tick period:
        //   pass 0: begin=100, end=1_100_100  (1.0001 ms)
        //   pass 1: begin=500_000, end=2_600_100  (2.1001 ms)
        // Sum of deltas = 3.1002 ms; wall-clock = last_end − first_begin
        //   = 2_600_100 − 100 = 2_600_000 ticks = 2.6 ms
        let ticks = [100u64, 1_100_100, 500_000, 2_600_100];
        let result = frame_wall_ms(&ticks, 1.0);
        assert!((result.unwrap() - 2.6).abs() < 1e-3, "got {:?}", result);
    }

    #[test]
    fn frame_wall_ms_skips_zero_ticks() {
        // Pass 0 was skipped (zeros); only pass 1 ran.
        // Wall-clock must not anchor to tick 0.
        let ticks = [0u64, 0, 1_000_000, 3_000_000];
        let result = frame_wall_ms(&ticks, 1.0);
        // 3_000_000 − 1_000_000 = 2_000_000 ticks × 1 ns = 2.0 ms
        assert!((result.unwrap() - 2.0).abs() < 1e-3, "got {:?}", result);
    }

    #[test]
    fn frame_wall_ms_begin_zero_end_nonzero_is_skipped() {
        // Explicit documentation of the begin==0 design choice: a pair whose
        // begin tick is zero but end tick is nonzero is treated as "pass did not
        // run this frame" and is excluded from the wall-clock reduction.
        //
        // Rationale: Metal leaves query slots at 0 when the GPU pass is not
        // issued. A begin==0 / end>0 pair would anchor the wall-clock min to
        // tick 0, making the total nonsensically large. The filter is therefore
        // intentional — a real GPU begin tick of exactly 0 is vanishingly
        // unlikely (it would require the GPU timestamp counter to sit at zero
        // at the precise moment the pass starts), and the consequence (one
        // dropped sample) is preferable to a corrupted wall-clock reading.
        //
        // Ticks: [0, 5_000_000,  1_000, 6_000_000]
        //   pass 0: begin=0 (unwritten) → skipped
        //   pass 1: begin=1_000, end=6_000_000 → 5.999 ms at 1 ns/tick
        //     (6_000_000 − 1_000 = 5_999_000 ticks × 1 ns / 1_000_000 ms)
        let ticks = [0u64, 5_000_000, 1_000, 6_000_000];
        let result = frame_wall_ms(&ticks, 1.0);
        assert!(
            (result.unwrap() - 5.999).abs() < 1e-3,
            "expected Some(5.999), got {:?}",
            result
        );
    }

    #[test]
    fn frame_wall_ms_returns_none_when_all_ticks_are_zero() {
        // All passes skipped — no valid data.
        let ticks = [0u64, 0, 0, 0];
        assert!(frame_wall_ms(&ticks, 1.0).is_none());
    }

    #[test]
    fn frame_wall_ms_returns_none_for_empty_slice() {
        assert!(frame_wall_ms(&[], 1.0).is_none());
    }

    #[test]
    fn frame_wall_ms_period_scales_result() {
        // One pass: begin=1, end=1001 ticks; period=41.7 ns → 41.7 µs = 0.0417 ms
        let ticks = [1u64, 1001];
        let result = frame_wall_ms(&ticks, 41.7);
        assert!((result.unwrap() - 0.0417).abs() < 1e-6, "got {:?}", result);
    }

    #[test]
    fn frame_wall_ms_single_valid_pass_equals_pass_millis() {
        // For a single non-zero pass, wall-clock == per-pass delta.
        let ticks = [500u64, 1_500_500];
        let wall = frame_wall_ms(&ticks, 1.0).unwrap();
        let per_pass = pass_millis(&ticks, 1.0)[0].unwrap();
        assert!((wall - per_pass).abs() < 1e-6);
    }

    #[test]
    fn frame_wall_ms_returns_none_when_all_pairs_invalid() {
        // Non-zero ticks but all end < begin (Metal bad samples, no zeros).
        let ticks = [5u64, 3, 9, 2];
        assert!(frame_wall_ms(&ticks, 1.0).is_none());
    }

    #[test]
    fn old_sum_overcounts_overlapping_passes() {
        // Demonstrate that summing per-pass deltas gives a LARGER number than
        // frame_wall_ms when passes overlap (which they do on TBDR).
        // pass 0: [100, 1_100_100]; pass 1: [500_000, 2_600_100]
        // pass 1 begins before pass 0 ends → overlap → sum > wall
        let ticks = [100u64, 1_100_100, 500_000, 2_600_100];
        let sum_ms: f32 = pass_millis(&ticks, 1.0).into_iter().flatten().sum();
        let wall_ms = frame_wall_ms(&ticks, 1.0).unwrap();
        assert!(
            sum_ms > wall_ms,
            "sum={sum_ms} should exceed wall={wall_ms}"
        );
    }
}
