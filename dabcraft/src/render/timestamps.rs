use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Measures one render pass's GPU time via pass-boundary timestamps.
/// Readback is async: `last_ms` lags a few frames behind.
pub struct GpuTimer {
    query_set: Option<wgpu::QuerySet>,
    resolve_buffer: wgpu::Buffer,
    read_buffer: wgpu::Buffer,
    map_done: Arc<AtomicBool>,
    pending: bool,
    pub last_ms: f32,
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device) -> Self {
        let enabled = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let query_set = enabled.then(|| {
            device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            })
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts resolve"),
            size: 16,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts read"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            query_set,
            resolve_buffer,
            read_buffer,
            map_done: Arc::new(AtomicBool::new(false)),
            pending: false,
            last_ms: 0.0,
        }
    }

    pub fn pass_writes(&self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if self.pending {
            return None;
        }
        self.query_set.as_ref().map(|qs| wgpu::RenderPassTimestampWrites {
            query_set: qs,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        })
    }

    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.pending {
            return;
        }
        if let Some(qs) = &self.query_set {
            encoder.resolve_query_set(qs, 0..2, &self.resolve_buffer, 0);
            encoder.copy_buffer_to_buffer(&self.resolve_buffer, 0, &self.read_buffer, 0, 16);
        }
    }

    pub fn after_submit(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.query_set.is_none() {
            return;
        }
        if !self.pending {
            let done = self.map_done.clone();
            self.read_buffer.map_async(wgpu::MapMode::Read, .., move |result| {
                if result.is_ok() {
                    done.store(true, Ordering::Release);
                }
            });
            self.pending = true;
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        if self.map_done.swap(false, Ordering::AcqRel) {
            {
                let data = self.read_buffer.get_mapped_range(..);
                let ts: &[u64] = bytemuck::cast_slice(&data);
                let ns = ts[1].wrapping_sub(ts[0]) as f32 * queue.get_timestamp_period();
                self.last_ms = ns / 1_000_000.0;
            }
            self.read_buffer.unmap();
            self.pending = false;
        }
    }
}
