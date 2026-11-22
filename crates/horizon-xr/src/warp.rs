//! Application SpaceWarp (`XR_FB_space_warp`): the game draws every other display frame
//! and the headset makes up the ones between from each eye's motion vectors and depth.
//!
//! This module owns the runtime's side: the motion vector and depth swapchains (at the
//! size the runtime recommends), handing their images to the renderer for each eye
//! (`acquire`, `release`), and the info chained to each eye's projection view (`info`).
//! The renderer fills them (gfx/space_warp.rs): the vectors are each point's motion in
//! the headset's LOCAL space since the last drawn frame (the world's under the player's
//! walking and turning, the hands', the mobs'), without the head's own motion, which the
//! runtime takes from the poses.
use crate::blit::EyeChain;
use ash::vk;
use openxr as xr;

/// Colour format of the motion vectors: signed 16-bit floats, as the runtime asks.
pub const MOTION_FORMAT: (vk::Format, wgpu::TextureFormat) =
    (vk::Format::R16G16B16A16_SFLOAT, wgpu::TextureFormat::Rgba16Float);

/// The motion vector and depth images of both eyes.
pub struct SpaceWarp {
    pub size: [u32; 2],
    pub depth_format: wgpu::TextureFormat,
    motion: [EyeChain; 2],
    depth: [EyeChain; 2],
    /// Each eye's images were filled this frame.
    pub filled: [bool; 2],
    /// Each eye's images are acquired (between `acquire` and `release`).
    acquired: [bool; 2],
    /// Distances (metres) at depth 0 and at depth 1: reversed Z, so the far plane first.
    pub range: (f32, f32),
    /// The runtime should not make up frames from this one (a teleport, a menu).
    pub skip: bool,
}

impl SpaceWarp {
    pub fn new(
        session: &xr::Session<xr::Vulkan>,
        device: &wgpu::Device,
        size: [u32; 2],
    ) -> Result<Self, String> {
        let offered = session
            .enumerate_swapchain_formats()
            .map_err(|e| format!("swapchain formats: {e}"))?;
        // Meta recommends D24S8; any depth the runtime takes will do.
        let depth_format = [
            (vk::Format::D24_UNORM_S8_UINT, wgpu::TextureFormat::Depth24PlusStencil8),
            (vk::Format::D32_SFLOAT, wgpu::TextureFormat::Depth32Float),
            (vk::Format::D16_UNORM, wgpu::TextureFormat::Depth16Unorm),
        ]
        .into_iter()
        .find(|(vk_format, _)| offered.contains(&(vk_format.as_raw() as u32)))
        .ok_or("no depth swapchain format")?;
        if !offered.contains(&(MOTION_FORMAT.0.as_raw() as u32)) {
            return Err("no RGBA16F swapchain format".into());
        }
        let motion = [
            EyeChain::with_usage(session, device, MOTION_FORMAT, size, false)?,
            EyeChain::with_usage(session, device, MOTION_FORMAT, size, false)?,
        ];
        let depth = [
            EyeChain::with_usage(session, device, depth_format, size, true)?,
            EyeChain::with_usage(session, device, depth_format, size, true)?,
        ];
        log::info!(
            "Horizon VR: space warp on, motion vectors and depth {}x{}, depth {:?}",
            size[0],
            size[1],
            depth_format.1
        );
        Ok(Self {
            size,
            depth_format: depth_format.1,
            motion,
            depth,
            filled: [false; 2],
            acquired: [false; 2],
            range: (1000.0, 0.05),
            skip: false,
        })
    }

    /// Eye `eye`'s motion vector and depth images to draw into, until `release`.
    pub fn acquire(&mut self, eye: usize) -> Result<(wgpu::TextureView, wgpu::TextureView), String> {
        if self.acquired[eye] {
            return Err("already acquired".into());
        }
        let motion = self.motion[eye].acquire()?.clone();
        let depth = match self.depth[eye].acquire() {
            Ok(view) => view.clone(),
            Err(e) => {
                let _ = self.motion[eye].release();
                return Err(e);
            }
        };
        self.acquired[eye] = true;
        Ok((motion, depth))
    }

    /// Gives eye `eye`'s images back; `filled`: the renderer drew both.
    pub fn release(&mut self, eye: usize, filled: bool) -> Result<(), String> {
        if !self.acquired[eye] {
            return Ok(());
        }
        self.acquired[eye] = false;
        let motion = self.motion[eye].release();
        self.depth[eye].release()?;
        motion?;
        self.filled[eye] = filled;
        Ok(())
    }

    /// The info to chain to eye `eye`'s projection view.
    pub fn info(&self, eye: usize) -> xr::sys::CompositionLayerSpaceWarpInfoFB {
        let rect = xr::Rect2Di {
            offset: xr::Offset2Di { x: 0, y: 0 },
            extent: xr::Extent2Di { width: self.size[0] as i32, height: self.size[1] as i32 },
        };
        let sub = |chain: &EyeChain| xr::sys::SwapchainSubImage {
            swapchain: chain.handle.as_raw(),
            image_rect: rect,
            image_array_index: 0,
        };
        xr::sys::CompositionLayerSpaceWarpInfoFB {
            ty: xr::sys::CompositionLayerSpaceWarpInfoFB::TYPE,
            next: std::ptr::null(),
            layer_flags: if self.skip {
                xr::sys::CompositionLayerSpaceWarpInfoFlagsFB::FRAME_SKIP
            } else {
                xr::sys::CompositionLayerSpaceWarpInfoFlagsFB::EMPTY
            },
            motion_vector_sub_image: sub(&self.motion[eye]),
            // The player's walking and turning are in the motion vectors themselves.
            app_space_delta_pose: xr::Posef::IDENTITY,
            depth_sub_image: sub(&self.depth[eye]),
            min_depth: 0.0,
            max_depth: 1.0,
            near_z: self.range.0,
            far_z: self.range.1,
        }
    }
}
