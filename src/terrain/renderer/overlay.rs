//! 作図・マーカー・航跡・覆域の頂点バッチ(`DrawVertex`のTriangleList)。

use super::uniforms::UniformSlot;
use crate::terrain::vertex::DrawVertex;

/// 作図の頂点バッファ1本(TriangleList)。容量が足りる間はGPUバッファを再利用する。
pub(super) struct VertexBatch {
    /// GPUバッファのラベル。
    label: &'static str,
    /// 頂点バッファ(容量は頂点データ以上。頂点が無ければNone)。
    buffer: Option<wgpu::Buffer>,
    /// 描く頂点数(バッファの容量ではなく、最後に`set`した頂点の数)。
    count: u32,
}

impl VertexBatch {
    /// 空のバッチ(バッファはまだ作らない)。
    pub(super) fn new(label: &'static str) -> Self {
        Self {
            label,
            buffer: None,
            count: 0,
        }
    }

    /// 頂点を差し替える。今のバッファに収まればそのまま書き込み、足りなければ2のべき乗に切り上げた
    /// 大きさで作り直す(空なら解放する)。
    pub(super) fn set(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[DrawVertex],
    ) {
        if vertices.is_empty() {
            self.buffer = None;
            self.count = 0;
            return;
        }
        let bytes = bytemuck::cast_slice(vertices);
        let required = bytes.len() as u64;
        if self
            .buffer
            .as_ref()
            .is_none_or(|buffer| buffer.size() < required)
        {
            self.buffer = Some(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(self.label),
                    // 少しずつ頂点が増える航跡でも、毎回確保し直さない。
                    size: required
                        .next_power_of_two()
                        .min(device.limits().max_buffer_size),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
            );
        }
        queue.write_buffer(self.buffer.as_ref().unwrap(), 0, bytes);
        self.count = vertices.len() as u32;
    }

    /// 描くものが無いか。
    pub(super) fn is_empty(&self) -> bool {
        self.buffer.is_none()
    }

    /// `pipeline`で頂点を描く(空なら何もしない)。
    /// `space`は作図の座標の種類ごとのuniform(`DrawUniform`)。
    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        space: &UniformSlot,
    ) {
        let Some(buffer) = self.buffer.as_ref() else {
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, space.bind_group(), &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}
