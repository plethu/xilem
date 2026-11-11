// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

//! CPU presentation for windows on hosts without a hardware GPU.
//!
//! Frames are rasterised with Vello CPU and copied into a `softbuffer` surface. This is used when
//! every wgpu adapter is a CPU implementation such as llvmpipe or WARP, where presenting through
//! wgpu only adds an emulated GPU copy, and when wgpu cannot create a surface at all.

use std::num::NonZeroU32;
use std::sync::Arc;

use masonry_imaging::PreparedFrame;
use masonry_imaging::vello_cpu::{self, Renderer};
use winit::window::Window as WindowHandle;

/// A window presented through `softbuffer` with Vello CPU frames.
pub(crate) struct SoftSurface {
    // The surface borrows the platform display from the context, so both are kept together.
    _context: softbuffer::Context<Arc<WindowHandle>>,
    surface: softbuffer::Surface<Arc<WindowHandle>, Arc<WindowHandle>>,
    renderer: Renderer,
    /// Reused frame pixels, tightly packed RGBA8.
    pixels: Vec<u8>,
}

impl SoftSurface {
    /// Create a CPU-presented surface for the window.
    pub(crate) fn new(handle: Arc<WindowHandle>) -> Result<Self, softbuffer::SoftBufferError> {
        let context = softbuffer::Context::new(handle.clone())?;
        let surface = softbuffer::Surface::new(&context, handle)?;
        let renderer =
            vello_cpu::new_headless_renderer().expect("creating a Vello CPU renderer cannot fail");
        Ok(Self {
            _context: context,
            surface,
            renderer,
            pixels: Vec::new(),
        })
    }

    /// Rasterise and present one frame.
    pub(crate) fn present(&mut self, window: &WindowHandle, frame: PreparedFrame<'_>) {
        let (Some(width), Some(height)) =
            (NonZeroU32::new(frame.width), NonZeroU32::new(frame.height))
        else {
            return;
        };
        if let Err(err) = vello_cpu::render_rgba8(&mut self.renderer, frame, &mut self.pixels) {
            tracing::error!("Couldn't render Masonry content with Vello CPU: {err}");
            return;
        }
        if let Err(err) = self.surface.resize(width, height) {
            tracing::error!("Couldn't resize the softbuffer surface: {err}");
            return;
        }
        let mut buffer = match self.surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(err) => {
                tracing::error!("Couldn't acquire a softbuffer frame: {err}");
                return;
            }
        };
        // softbuffer pixels are 0x00RRGGBB; window frames are opaque.
        for (pixel, rgba) in buffer.iter_mut().zip(self.pixels.chunks_exact(4)) {
            *pixel = (u32::from(rgba[0]) << 16) | (u32::from(rgba[1]) << 8) | u32::from(rgba[2]);
        }
        window.pre_present_notify();
        if let Err(err) = buffer.present() {
            tracing::error!("Couldn't present the softbuffer frame: {err}");
        }
    }
}

/// Environment variable that forces a backend; `softbuffer` selects this presenter.
const OVERRIDE: &str = "MASONRY_IMAGING_BACKEND";

/// Whether windows should present through `softbuffer` rather than wgpu.
///
/// True when `MASONRY_IMAGING_BACKEND=softbuffer`, or when wgpu finds adapters and all of them are
/// CPU implementations. Without adapters this returns false, so surface creation fails and falls
/// back with its own reason. The choice is logged once.
pub(crate) fn prefer_softbuffer(instance: &wgpu::Instance) -> bool {
    match std::env::var(OVERRIDE).ok().as_deref() {
        Some("softbuffer") => {
            tracing::info!(
                target: "masonry_imaging::selection",
                backend = "imaging_vello_cpu",
                reason = "override",
                presenter = "softbuffer",
                "Selected Masonry imaging backend"
            );
            return true;
        }
        Some("vello" | "vello_cpu") => return false,
        _ => {}
    }
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    let Some(first) = adapters.first() else {
        return false;
    };
    if adapters
        .iter()
        .any(|adapter| adapter.get_info().device_type != wgpu::DeviceType::Cpu)
    {
        return false;
    }
    let info = first.get_info();
    tracing::info!(
        target: "masonry_imaging::selection",
        backend = "imaging_vello_cpu",
        reason = "software adapter",
        presenter = "softbuffer",
        adapter = %info.name,
        device_type = ?info.device_type,
        "Selected Masonry imaging backend"
    );
    true
}
