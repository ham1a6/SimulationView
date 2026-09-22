//! 3Dモデル(`terrain::models`)のGPU側: モデルごとの頂点・インデックスと、インスタンス(1機ごとの変換行列)のバッファ、
//! 描画パイプライン(`model.wgsl`)。モデルの追加・削除・インスタンスの更新は`TerrainRenderer`の`set_model`等が、
//! 描画は`encode_main_pass`が呼ぶ。パイプラインは頂点バッファ2本(頂点・インスタンス)を使うので、
//! 1本しか持てない共通の`pipelines::create_pipeline`は使わず、ここで作る(設定の中身は同じ: MSAA・反転Zの深度・カリングなし)。

use std::collections::HashMap;

use wgpu::util::DeviceExt;

use super::overlay::DrawSpace;
use super::targets::SAMPLE_COUNT;
use crate::terrain::models::types::{ModelInstance, ModelMesh, ModelVertex};

/// 頂点(`ModelVertex`)のシェーダー入力(`model.wgsl`の`VertexInput`のlocation 0〜2)。
pub(super) const MODEL_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];
/// インスタンス(`ModelInstance`)のシェーダー入力(location 3〜7。行列の4列と色)。
pub(super) const MODEL_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4];

/// インスタンスバッファの最小の容量(個)。
const MIN_INSTANCE_CAPACITY: usize = 16;

/// モデル1つ分のGPUバッファ。
struct ModelGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    num_indices: u32,
    /// インスタンスのバッファ(容量は`capacity`個。足りなくなったら大きく作り直す)。
    instance_buffer: Option<wgpu::Buffer>,
    capacity: usize,
    /// このフレームに描くインスタンスの数。
    count: u32,
}

pub(super) struct ModelBatch {
    pipeline: wgpu::RenderPipeline,
    models: HashMap<String, ModelGpu>,
}

impl ModelBatch {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        draw_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("model_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../models/model.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("model_pipeline_layout"),
            bind_group_layouts: &[Some(draw_bind_group_layout)],
            immediate_size: 0,
        });
        let buffers = [
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<ModelVertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &MODEL_VERTEX_ATTRIBUTES,
            },
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<ModelInstance>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &MODEL_INSTANCE_ATTRIBUTES,
            },
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("model_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &buffers.map(Some),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            // 裏面カリングは使わない(両面のモデルがある。パイプライン共通の設定と同じ)。
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            // 不透明なので深度を書く。反転Zなので「より近ければ描く」はGreater。
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            models: HashMap::new(),
        }
    }

    /// モデルを登録する(同じキーがあれば置き換える。インスタンスは空になる)。
    pub(super) fn set_model(&mut self, device: &wgpu::Device, key: &str, mesh: &ModelMesh) {
        if mesh.indices.is_empty() {
            self.models.remove(key);
            return;
        }
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("model_vertex_buffer"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("model_index_buffer"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.models.insert(
            key.to_string(),
            ModelGpu {
                vertex_buffer,
                index_buffer,
                num_indices: mesh.indices.len() as u32,
                instance_buffer: None,
                capacity: 0,
                count: 0,
            },
        );
    }

    pub(super) fn remove_model(&mut self, key: &str) {
        self.models.remove(key);
    }

    /// このフレームに描くインスタンスを差し替える。`instances`に無いモデルは、何も描かない。登録されていないキーは無視する。
    pub(super) fn set_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &HashMap<String, Vec<ModelInstance>>,
    ) {
        for (key, model) in self.models.iter_mut() {
            let list: &[ModelInstance] = instances.get(key).map_or(&[], |v| v.as_slice());
            model.count = list.len() as u32;
            if list.is_empty() {
                continue;
            }
            if list.len() > model.capacity {
                model.capacity = list.len().next_power_of_two().max(MIN_INSTANCE_CAPACITY);
                model.instance_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("model_instance_buffer"),
                    size: (model.capacity * std::mem::size_of::<ModelInstance>())
                        as wgpu::BufferAddress,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            if let Some(buffer) = model.instance_buffer.as_ref() {
                queue.write_buffer(buffer, 0, bytemuck::cast_slice(list));
            }
        }
    }

    /// 描くインスタンスが1つも無いか。
    pub(super) fn is_empty(&self) -> bool {
        self.models.values().all(|m| m.count == 0)
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, space: &DrawSpace) {
        if self.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, space.bind_group(), &[]);
        for model in self.models.values().filter(|m| m.count > 0) {
            let Some(instances) = model.instance_buffer.as_ref() else {
                continue;
            };
            pass.set_vertex_buffer(0, model.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, instances.slice(..));
            pass.set_index_buffer(model.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..model.num_indices, 0, 0..model.count);
        }
    }
}
