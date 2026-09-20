//! 作図・マーカー・航跡・覆域の頂点バッチ(`DrawVertex`のTriangleList)と、座標の種類ごとのuniform。

use wgpu::util::DeviceExt;

use super::uniforms::{uniform_bind_group, DrawUniform};
use crate::terrain::vertex::DrawVertex;

/// 作図の座標の種類1つ分のuniformとbind group。
pub(super) struct DrawSpace {
    pub buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl DrawSpace {
    pub(super) fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, label: &str) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: std::mem::size_of::<DrawUniform>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = uniform_bind_group(device, label, layout, &buffer);
        Self { buffer, bind_group }
    }
}

/// 作図の頂点バッファ1本(TriangleList)。空ならバッファを持たない。
pub(super) struct VertexBatch {
    buffer: Option<wgpu::Buffer>,
    count: u32,
}

impl VertexBatch {
    pub(super) fn empty() -> Self {
        Self { buffer: None, count: 0 }
    }

    pub(super) fn set(&mut self, device: &wgpu::Device, label: &str, vertices: &[DrawVertex]) {
        if vertices.is_empty() {
            *self = Self::empty();
            return;
        }
        self.buffer = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }));
        self.count = vertices.len() as u32;
    }

    pub(super) fn is_empty(&self) -> bool {
        self.buffer.is_none()
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline, space: &DrawSpace) {
        let Some(buffer) = self.buffer.as_ref() else {
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &space.bind_group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}
