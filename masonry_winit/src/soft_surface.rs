// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

//! CPU presentation for windows on hosts without a hardware GPU.
//!
//! Frames are rasterised with Vello CPU and copied into a `softbuffer` surface. This is used when
//! every wgpu adapter is a CPU implementation such as llvmpipe or WARP, where presenting through
//! wgpu only adds an emulated GPU copy, and when wgpu cannot create a surface at all.

use std::num::NonZeroU32;
use std::sync::Arc;

use masonry_core::kurbo::Rect;
use masonry_imaging::vello_cpu::IncrementalRenderer;
use masonry_imaging::{PixelRect, PreparedFrame};
use winit::window::Window as WindowHandle;

/// A window presented through `softbuffer` with Vello CPU frames.
///
/// Frames are rasterised into a persistent framebuffer, re-rasterising only the region Masonry
/// reports as damaged, and presented with that damage so the window system only copies and
/// recomposites what changed.
pub(crate) struct SoftSurface {
    // The surface borrows the platform display from the context, so both are kept together.
    _context: softbuffer::Context<Arc<WindowHandle>>,
    surface: softbuffer::Surface<Arc<WindowHandle>, Arc<WindowHandle>>,
    renderer: IncrementalRenderer,
    /// Size the surface was last resized to.
    size: Option<(NonZeroU32, NonZeroU32)>,
}

impl SoftSurface {
    /// Create a CPU-presented surface for the window.
    pub(crate) fn new(handle: Arc<WindowHandle>) -> Result<Self, softbuffer::SoftBufferError> {
        let context = softbuffer::Context::new(handle.clone())?;
        let surface = softbuffer::Surface::new(&context, handle)?;
        let renderer =
            IncrementalRenderer::new().expect("creating a Vello CPU renderer cannot fail");
        Ok(Self {
            _context: context,
            surface,
            renderer,
            size: None,
        })
    }

    /// Rasterise and present one frame.
    ///
    /// `damage` is the window region Masonry reports as changed since the previous frame, in
    /// logical pixels, or `None` to repaint everything. `expose` presents the whole frame even if
    /// only part of it was re-rasterised, for redraws the window system requested because the
    /// window's contents were lost.
    pub(crate) fn present(
        &mut self,
        window: &WindowHandle,
        frame: PreparedFrame<'_>,
        damage: Option<&[Rect]>,
        expose: bool,
    ) {
        let (Some(width), Some(height)) =
            (NonZeroU32::new(frame.width), NonZeroU32::new(frame.height))
        else {
            return;
        };
        let updated = match self.renderer.render(frame, damage) {
            Ok(updated) => updated,
            Err(err) => {
                tracing::error!("Couldn't render Masonry content with Vello CPU: {err}");
                self.renderer.invalidate();
                return;
            }
        };
        if let Err(err) = self.present_framebuffer(window, (width, height), updated, expose) {
            tracing::error!("Couldn't present the softbuffer frame: {err}");
            // The window may now show something other than the framebuffer.
            self.renderer.invalidate();
        }
    }

    fn present_framebuffer(
        &mut self,
        window: &WindowHandle,
        (width, height): (NonZeroU32, NonZeroU32),
        updated: Option<Vec<PixelRect>>,
        expose: bool,
    ) -> Result<(), softbuffer::SoftBufferError> {
        if self.size != Some((width, height)) {
            self.surface.resize(width, height)?;
            self.size = Some((width, height));
        }
        let updated = updated.filter(|_| !expose);
        if updated.as_ref().is_some_and(Vec::is_empty) {
            // Nothing changed and the window still shows the previous frame.
            return Ok(());
        }
        let mut buffer = self.surface.buffer_mut()?;
        let framebuffer = self.renderer.framebuffer();
        // The buffer holds the previously presented frame only when its age is 1; otherwise it
        // is new or older, so the whole framebuffer is copied. Presentation damage is relative to
        // the previously presented frame either way.
        match &updated {
            Some(rects) if buffer.age() == 1 => {
                let stride = width.get() as usize;
                for rect in rects {
                    for row in rect.y..rect.bottom() {
                        let start = row as usize * stride + rect.x as usize;
                        let end = start + rect.width as usize;
                        buffer[start..end].copy_from_slice(&framebuffer[start..end]);
                    }
                }
            }
            _ => buffer.copy_from_slice(framebuffer),
        }
        window.pre_present_notify();
        match updated {
            Some(rects) => {
                let damage: Vec<softbuffer::Rect> = rects
                    .iter()
                    .filter_map(|rect| {
                        Some(softbuffer::Rect {
                            x: rect.x,
                            y: rect.y,
                            width: NonZeroU32::new(rect.width)?,
                            height: NonZeroU32::new(rect.height)?,
                        })
                    })
                    .collect();
                buffer.present_with_damage(&damage)
            }
            None => buffer.present(),
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
