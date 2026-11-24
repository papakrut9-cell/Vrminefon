//! VR's multisampling: the world pass's first part (sky, solid terrain, block entities and
//! entities) draws into multisampled colour and depth targets, its colour resolved into the
//! world target at the end of that part, its depth copied (sample 0) into the world's depth
//! for the rest of the world pass (translucent terrain, particles, clouds, weather...),
//! which draws on single-sampled as before. Only frames whose world pass is that one plain
//! pass take it (`Gfx::render_screen`'s `msaa_frame`).
use super::SolidPipeline;

const COPY_SHADER: &str = r"
@group(0) @binding(0) var depth_ms: texture_depth_multisampled_2d;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    return textureLoad(depth_ms, vec2<i32>(p.xy), 0);
}
";

/// The multisampled copies of the first part's pipelines.
pub(super) struct Pipelines {
    pub sky_base: wgpu::RenderPipeline,
    pub sunrise: wgpu::RenderPipeline,
    pub dark_disc: wgpu::RenderPipeline,
    pub celestial: wgpu::RenderPipeline,
    pub static_entity: wgpu::RenderPipeline,
    pub entity: wgpu::RenderPipeline,
    pub solid: SolidPipeline,
    pub solid_opaque: SolidPipeline,
    pub solid_cutout: SolidPipeline,
    /// The flat solid draws' (`super::flat`): whole, opaque, cutout.
    pub flat: Option<[SolidPipeline; 3]>,
    /// The flat translucent terrain's: water, ice and glass drawn multisampled too.
    pub flat_translucent: Option<wgpu::RenderPipeline>,
}

struct Targets {
    size: [u32; 2],
    format: wgpu::TextureFormat,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    group: wgpu::BindGroup,
}

/// VR's single-pass stereo (`super::stereo`): both eyes' multisampled colour and depth as
/// two-layer arrays, the colour resolved into a two-layer array each eye copies its own
/// layer from, and a depth copy bind group per layer.
struct StereoTargets {
    size: [u32; 2],
    format: wgpu::TextureFormat,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolved: wgpu::Texture,
    resolved_view: wgpu::TextureView,
    groups: [wgpu::BindGroup; 2],
}

pub(super) struct Msaa {
    pub samples: u32,
    pub pipelines: Pipelines,
    /// The multiview copies of `pipelines` (`super::stereo`), where the device has them.
    pub stereo: Option<Pipelines>,
    copy: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    targets: Option<Targets>,
    stereo_targets: Option<StereoTargets>,
}

impl Msaa {
    pub(super) fn new(
        device: &wgpu::Device,
        samples: u32,
        pipelines: Pipelines,
        stereo: Option<Pipelines>,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon VR MSAA depth copy"),
            source: wgpu::ShaderSource::Wgsl(COPY_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon VR MSAA depth copy"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: true,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon VR MSAA depth copy"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let copy = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Horizon VR MSAA depth copy"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { samples, pipelines, stereo, copy, layout, targets: None, stereo_targets: None }
    }

    /// Targets the size of the world's (`size`), in its colour `format`.
    pub(super) fn fit(&mut self, device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat) {
        if self.targets.as_ref().is_some_and(|t| t.size == size && t.format == format) {
            return;
        }
        let texture = |label, format, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: self.samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let color = texture("Horizon VR MSAA colour", format, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let depth = texture(
            "Horizon VR MSAA depth",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon VR MSAA depth copy"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&depth) }],
        });
        self.targets = Some(Targets { size, format, color, depth, group });
    }

    /// The stereo targets the size of the world's (`size`), in its colour `format`.
    pub(super) fn fit_stereo(&mut self, device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat) {
        if self.stereo_targets.as_ref().is_some_and(|t| t.size == size && t.format == format) {
            return;
        }
        let texture = |label, format, usage, samples| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 2 },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let array = |texture: &wgpu::Texture| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let color = texture(
            "Horizon VR stereo MSAA colour",
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            self.samples,
        );
        let depth = texture(
            "Horizon VR stereo MSAA depth",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            self.samples,
        );
        let resolved = texture(
            "Horizon VR stereo world",
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            1,
        );
        let groups = [0u32, 1].map(|layer| {
            let view = depth.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Horizon VR stereo depth copy"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
            })
        });
        self.stereo_targets = Some(StereoTargets {
            size,
            format,
            color: array(&color),
            depth: array(&depth),
            resolved_view: array(&resolved),
            resolved,
            groups,
        });
    }

    /// The stereo pass's multisampled colour and depth arrays and the colour's resolve
    /// array (after `fit_stereo`).
    pub(super) fn stereo_views(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView)> {
        self.stereo_targets.as_ref().map(|t| (&t.color, &t.depth, &t.resolved_view))
    }

    /// Eye `layer`'s part of the stereo pass: its resolved colour into the world target
    /// `world` and its depth's sample 0 into the world's `depth`.
    pub(super) fn take_stereo_layer(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        layer: u32,
        world: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        timing: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let Some(t) = &self.stereo_targets else { return };
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &t.resolved,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: world.texture(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d { width: t.size[0], height: t.size[1], depth_or_array_layers: 1 },
        );
        self.copy_depth_from(encoder, depth, &t.groups[layer as usize], timing);
    }

    /// The multisampled colour and depth (after `fit`).
    pub(super) fn views(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView)> {
        self.targets.as_ref().map(|t| (&t.color, &t.depth))
    }

    /// The multisampled depth's sample 0 into the world's `depth`.
    /// `timing`: its GPU time, counted with the frame's passes (VR's frame pacing reads them).
    pub(super) fn copy_depth(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        timing: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let Some(t) = &self.targets else { return };
        self.copy_depth_from(encoder, depth, &t.group, timing);
    }

    fn copy_depth_from(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        group: &wgpu::BindGroup,
        timing: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon VR MSAA depth copy"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: timing,
            ..Default::default()
        });
        pass.set_pipeline(&self.copy);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}
