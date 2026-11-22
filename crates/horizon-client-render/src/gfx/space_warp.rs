//! VR's Application SpaceWarp, the renderer's side (horizon-xr's `warp` owns the runtime's):
//! each eye's motion vectors and depth, at the size the runtime asks for, after the eye's
//! frame.
//!
//! A motion vector is where a pixel's point is now minus where it was at the last drawn
//! frame, both in this frame's NDC: every point is seen through this frame's eye, so the
//! head's own motion stays out (the runtime takes it from the poses), and both positions
//! are kept in the headset's LOCAL space, so the player's walking and turning, which move
//! the world through LOCAL space, are in (`horizon_xr::space_warp_rig`).
//!
//! - The world (terrain, sky, what does not move on its own) comes from the eye's depth:
//!   each pixel's point, where it sat in LOCAL space at the last frame.
//! - What moves on its own is drawn over it with each vertex's last position: the hands
//!   and what they hold (the same vertices frame to frame), and the entities the game
//!   tags with their ids (`Frame::entity_motion`), matched to last frame's by id and shape
//!   as frame generation does. Translucent items stay out (Meta: opaque only).
//!
//! `HORIZON_XR_SW_FLIPY=0` keeps wgpu's NDC (y up) instead of Vulkan's (y down) in the
//! vectors, `HORIZON_XR_SW_SCALE` scales them (0 turns the made-up frames into plain
//! reprojection), for checks on the headset.
use crate::mesher::Vertex;
use glam::{DVec3, Mat4, Quat, Vec3};
use std::collections::HashMap;

/// The id the hands' vertices are tracked under (entity ids are the game's, never this).
const HANDS: u64 = u64::MAX;

const SHADER: &str = r"
struct Params {
    // This frame's eye: camera-relative world to clip, and back.
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    // Last frame's camera-relative point q kept at its LOCAL place: rig * q (relative to
    // this frame's camera).
    rig: mat4x4<f32>,
    // xyz: this frame's camera less the last one's; w unused.
    moved: vec4<f32>,
    // x: 1 flips the vectors' y to Vulkan's NDC; y: the vectors' scale; zw: the eye
    // depth's size.
    misc: vec4<f32>,
    // xy: the motion image's size; z: the footprint's depth (0 nearest, 1 centre, 2
    // farthest; `HORIZON_XR_SW_DEPTH`).
    target_size: vec4<f32>,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth: texture_depth_2d;

fn vector(now: vec4<f32>, then: vec4<f32>) -> vec4<f32> {
    if then.w <= 1e-6 || now.w <= 1e-6 {
        return vec4<f32>(0.0);
    }
    var v = (now.xyz / now.w - then.xyz / then.w) * params.misc.y;
    if params.misc.x > 0.5 {
        v.y = -v.y;
    }
    return vec4<f32>(v, 0.0);
}

struct ScreenOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex fn vs_screen(@builtin(vertex_index) i: u32) -> ScreenOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: ScreenOut;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

struct WorldOut {
    @location(0) motion: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

// The world: each pixel's point from the depth (reversed Z; 0 is the sky, a point at
// infinity, which only the rig's turn moves).
// The depth over the motion texel's whole footprint in the eye's depth, its nearest point:
// one sample of a bush or of leaves' cut-out holes picks the leaf or what is behind it at
// random from frame to frame, and the made-up frames then smear the foliage; the nearest
// over the footprint treats a bush as solid, steady from frame to frame.
fn footprint_depth(uv: vec2<f32>) -> f32 {
    let size = params.misc.zw;
    let step = size / params.target_size.xy;
    let corner = uv * size - step * 0.5;
    let mode = params.target_size.z;
    if mode > 0.5 && mode < 1.5 {
        return textureLoad(depth, vec2<i32>(clamp(uv * size, vec2<f32>(0.0), size - 1.0)), 0);
    }
    var nearest = 0.0;
    var farthest = 1.0;
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            let at = corner + step * (vec2<f32>(f32(x), f32(y)) + 0.5) / 4.0;
            let texel = vec2<i32>(clamp(at, vec2<f32>(0.0), size - 1.0));
            let d = textureLoad(depth, texel, 0);
            nearest = max(nearest, d);
            farthest = min(farthest, d);
        }
    }
    if mode > 1.5 {
        return farthest;
    }
    return nearest;
}

@fragment fn fs_world(in: ScreenOut) -> WorldOut {
    let d = footprint_depth(in.uv);
    let ndc = vec4<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, d, 1.0);
    let p = params.inverse_view_proj * ndc;
    let q = vec4<f32>(p.xyz + params.moved.xyz * p.w, p.w);
    var out: WorldOut;
    out.motion = vector(params.view_proj * p, params.view_proj * (params.rig * q));
    out.depth = d;
    return out;
}

struct ObjectOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) now: vec4<f32>,
    @location(1) then: vec4<f32>,
};

@vertex fn vs_object(@location(0) position: vec3<f32>, @location(1) previous: vec3<f32>) -> ObjectOut {
    var out: ObjectOut;
    out.clip = params.view_proj * vec4<f32>(position, 1.0);
    out.now = out.clip;
    out.then = params.view_proj * (params.rig * vec4<f32>(previous, 1.0));
    return out;
}

// What moves on its own, only where the eye's depth shows it (not behind something).
@fragment fn fs_object(in: ObjectOut) -> @location(0) vec4<f32> {
    let size = params.misc.zw;
    let texel = vec2<i32>(clamp(in.clip.xy * (size / params.target_size.xy), vec2<f32>(0.0), size - 1.0));
    let stored = textureLoad(depth, texel, 0);
    if in.clip.z < stored - (stored * 0.01 + 1e-6) {
        discard;
    }
    return vector(in.now, in.then);
}
";

/// `SHADER`'s `Params`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    view_proj: [[f32; 4]; 4],
    inverse_view_proj: [[f32; 4]; 4],
    rig: [[f32; 4]; 4],
    moved: [f32; 4],
    misc: [f32; 4],
    target_size: [f32; 4],
}

/// A vertex now and where it was at the last drawn frame (relative to that frame's camera).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MotionVertex {
    position: [f32; 3],
    previous: [f32; 3],
}

pub(super) struct SpaceWarp {
    depth_format: wgpu::TextureFormat,
    world: wgpu::RenderPipeline,
    object: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: [wgpu::Buffer; 2],
    vertices: wgpu::Buffer,
    /// Triangles of what moves on its own, matched this frame.
    staged: Vec<MotionVertex>,
    /// Last frame's vertices by entity id (`HANDS` for the hands).
    tracked: HashMap<u64, Vec<Vertex>>,
    camera: Option<DVec3>,
    /// This frame's camera less the last one's.
    moved: Vec3,
    flip_y: bool,
    scale: f32,
    /// `HORIZON_XR_SW_DEPTH`: `centre` or `far` instead of the footprint's nearest depth.
    depth_mode: f32,
}

impl SpaceWarp {
    pub(super) fn new(device: &wgpu::Device, depth_format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon VR space warp"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon VR space warp"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Horizon VR space warp"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let motion_target = [Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba16Float,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let world = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Horizon VR space warp world"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_screen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_world"),
                compilation_options: Default::default(),
                targets: &motion_target,
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let object = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Horizon VR space warp objects"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_object"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MotionVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_object"),
                compilation_options: Default::default(),
                targets: &motion_target,
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let params = [0, 1].map(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Horizon VR space warp"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        Self {
            depth_format,
            world,
            object,
            layout,
            params,
            vertices: vertex_buffer(device, 1 << 14),
            staged: Vec::new(),
            tracked: HashMap::new(),
            camera: None,
            moved: Vec3::ZERO,
            flip_y: std::env::var("HORIZON_XR_SW_FLIPY").as_deref() != Ok("0"),
            scale: std::env::var("HORIZON_XR_SW_SCALE")
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(1.0),
            depth_mode: match std::env::var("HORIZON_XR_SW_DEPTH").as_deref() {
                Ok("centre") | Ok("center") => 1.0,
                Ok("far") => 2.0,
                _ => 0.0,
            },
        }
    }

    pub(super) fn depth_format(&self) -> wgpu::TextureFormat {
        self.depth_format
    }

    /// Before the eyes, each drawn frame: this frame's entities (camera-relative, tagged by
    /// `spans` as `Frame::entity_motion`) and hands, matched to last frame's; `camera`.
    /// Returns whether the camera jumped (a teleport: no frames should be made up).
    pub(super) fn track(&mut self, entities: &[Vertex], spans: &[(u32, u64)], hands: &[Vertex], camera: DVec3) -> bool {
        self.staged.clear();
        let mut tracked = HashMap::with_capacity(self.tracked.len());
        let mut follow = |id: u64, now: &[Vertex], staged: &mut Vec<MotionVertex>| {
            if now.is_empty() {
                return;
            }
            if let Some(then) = self.tracked.remove(&id)
                && then.len() == now.len()
                && then.iter().zip(now).all(|(a, b)| a.uv == b.uv && a.layer == b.layer)
            {
                let translucent = |v: &Vertex| {
                    matches!(
                        v.layer,
                        super::entity_batch::TRANSLUCENT_ITEM_OVERLAY
                            | super::entity_batch::TRANSLUCENT_GLINT_ITEM_OVERLAY
                    )
                };
                for q in (0..now.len() / 4).map(|q| q * 4) {
                    if translucent(&now[q]) {
                        continue;
                    }
                    staged.extend([q, q + 1, q + 2, q, q + 2, q + 3].map(|i| MotionVertex {
                        position: now[i].pos,
                        previous: then[i].pos,
                    }));
                }
            }
            tracked.insert(id, now.to_vec());
        };
        for (i, &(start, id)) in spans.iter().enumerate() {
            let end = spans.get(i + 1).map_or(entities.len(), |s| s.0 as usize);
            if id == 0 {
                continue;
            }
            if let Some(now) = entities.get(start as usize..end) {
                follow(id, now, &mut self.staged);
            }
        }
        follow(HANDS, hands, &mut self.staged);
        self.tracked = tracked;
        let moved = self.camera.map_or(DVec3::ZERO, |c| camera - c);
        self.camera = Some(camera);
        self.moved = moved.as_vec3();
        moved.length() > 4.0
    }

    /// Forget last frame's tracking (a frame the eyes did not draw, a menu).
    pub(super) fn reset(&mut self) {
        self.tracked.clear();
        self.camera = None;
    }

    /// Eye `eye`'s motion vectors and depth into `motion` and `target_depth` (the runtime's
    /// images, `size`), from `depth` (the eye's depth after its frame) and `view_proj`
    /// (the eye's), with `rig` (`horizon_xr::space_warp_rig`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fill(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        eye: usize,
        depth: &wgpu::TextureView,
        view_proj: Mat4,
        rig: (Quat, Vec3),
        motion: &wgpu::TextureView,
        target_depth: &wgpu::TextureView,
    ) {
        let size = depth.texture().size();
        let rig = Mat4::from_translation(rig.1) * Mat4::from_quat(rig.0);
        let params = Params {
            view_proj: view_proj.to_cols_array_2d(),
            inverse_view_proj: view_proj.inverse().to_cols_array_2d(),
            rig: rig.to_cols_array_2d(),
            moved: [self.moved.x, self.moved.y, self.moved.z, 1.0],
            misc: [
                if self.flip_y { 1.0 } else { 0.0 },
                self.scale,
                size.width as f32,
                size.height as f32,
            ],
            target_size: {
                let t = motion.texture().size();
                [t.width as f32, t.height as f32, self.depth_mode, 0.0]
            },
        };
        queue.write_buffer(&self.params[eye], 0, bytemuck::bytes_of(&params));
        let bytes: &[u8] = bytemuck::cast_slice(&self.staged);
        if eye == 0 && !bytes.is_empty() {
            if bytes.len() as u64 > self.vertices.size() {
                self.vertices = vertex_buffer(device, (bytes.len() as u64).next_power_of_two());
            }
            queue.write_buffer(&self.vertices, 0, bytes);
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon VR space warp"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params[eye].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth) },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Horizon VR space warp"),
        });
        {
            let stencil = self.depth_format.has_stencil_aspect().then_some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0),
                store: wgpu::StoreOp::Store,
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Horizon VR space warp"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: motion,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: target_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: stencil,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &group, &[]);
            pass.set_pipeline(&self.world);
            pass.draw(0..3, 0..1);
            if !self.staged.is_empty() {
                pass.set_pipeline(&self.object);
                pass.set_vertex_buffer(0, self.vertices.slice(..bytes.len() as u64));
                pass.draw(0..self.staged.len() as u32, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
    }
}

fn vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Horizon VR space warp vertices"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_validates() {
        let module = naga::front::wgsl::parse_str(SHADER).unwrap();
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap();
    }

    /// The world's vectors: still when nothing moved; a step forward moves a point ahead
    /// toward the eye (its NDC grows away from the centre), the sky only by the turn.
    #[test]
    fn rig_and_camera_motion_reproject_a_world_point() {
        let projection = super::super::off_centre_projection([-1.0, 1.0, 1.0, -1.0], 100.0);
        let view_proj = projection;
        let p = glam::Vec4::new(1.0, 0.5, -10.0, 1.0);
        let still = Mat4::IDENTITY * (p + glam::Vec4::ZERO);
        let ndc = |c: glam::Vec4| c.truncate() / c.w;
        assert_eq!(ndc(view_proj * still), ndc(view_proj * p));
        // The camera went 1 forward (-Z): last frame the point was 11 ahead.
        let moved = Vec3::new(0.0, 0.0, -1.0);
        let q = glam::Vec4::new(p.x + moved.x, p.y + moved.y, p.z + moved.z, 1.0);
        let now = ndc(view_proj * p);
        let then = ndc(view_proj * q);
        assert!(now.x > then.x && now.y > then.y, "{now} {then}");
    }
}
