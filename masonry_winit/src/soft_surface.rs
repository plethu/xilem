// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

//! CPU presentation for windows when wgpu cannot create a surface.
//!
//! Frames are rasterised with Vello CPU and copied into a `softbuffer` surface, so a window still
//! renders on hosts without a usable Vulkan, Metal, D3D or GL driver.

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
