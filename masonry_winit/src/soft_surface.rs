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
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window as WindowHandle;

/// A window presented through `softbuffer` with Vello CPU frames.
///
/// Frames are rasterised into a persistent framebuffer, re-rasterising only the region Masonry
/// reports as damaged. Where the window system keeps the window's contents, the frame is presented
/// with that damage so it only copies and recomposites what changed.
pub(crate) struct SoftSurface {
    // The surface borrows the platform display from the context, so both are kept together.
    _context: softbuffer::Context<Arc<WindowHandle>>,
    surface: softbuffer::Surface<Arc<WindowHandle>, Arc<WindowHandle>>,
    renderer: IncrementalRenderer,
    /// Size the surface was last resized to.
    size: Option<(NonZeroU32, NonZeroU32)>,
    /// Whether the window system keeps the presented frame, so damage-limited presents are safe.
    retains_contents: bool,
    /// Whether the window may show something other than the framebuffer, so the next present must
    /// cover the whole window. Cleared only by a successful full present.
    full_present_pending: bool,
}

/// How one frame reaches the window.
#[derive(Debug, PartialEq, Eq)]
enum Presentation<'a> {
    /// The window already shows the framebuffer.
    Skip,
    /// Present the whole framebuffer.
    Full,
    /// Present only these rectangles.
    Damage(&'a [PixelRect]),
}

/// Chooses how to present a frame.
///
/// `updated` is what the renderer re-rasterised (`None` for the whole frame); `full_present` is set
/// when the window system may have lost or asked for the window's contents. Without retained
/// contents, every present covers the whole window: the window system can discard contents at any
/// time, and winit merges its redraw requests with Masonry's, so a request cannot be told apart.
fn presentation(
    updated: Option<&[PixelRect]>,
    full_present: bool,
    retains_contents: bool,
) -> Presentation<'_> {
    match updated {
        _ if full_present || !retains_contents => Presentation::Full,
        None => Presentation::Full,
        Some([]) => Presentation::Skip,
        Some(rects) => Presentation::Damage(rects),
    }
}

/// Whether only `updated` needs copying into a softbuffer buffer of the given age.
///
/// A buffer of age 1 holds the previously presented frame, which matches the framebuffer outside
/// what was just re-rasterised. Any other buffer is new or older and gets the whole framebuffer.
fn copy_only_updated(updated: Option<&[PixelRect]>, buffer_age: u8) -> bool {
    updated.is_some() && buffer_age == 1
}

impl SoftSurface {
    /// Create a CPU-presented surface for the window.
    pub(crate) fn new(handle: Arc<WindowHandle>) -> Result<Self, softbuffer::SoftBufferError> {
        // Wayland compositors keep the attached buffer, so damage is only a hint about what
        // changed. X11 and Win32 can discard window contents and ask for a redraw.
        let retains_contents = matches!(
            handle.window_handle().map(|handle| handle.as_raw()),
            Ok(RawWindowHandle::Wayland(_))
        );
        let context = softbuffer::Context::new(handle.clone())?;
        let surface = softbuffer::Surface::new(&context, handle)?;
        let renderer =
            IncrementalRenderer::new().expect("creating a Vello CPU renderer cannot fail");
        Ok(Self {
            _context: context,
            surface,
            renderer,
            size: None,
            retains_contents,
            full_present_pending: true,
        })
    }

    /// Rasterise and present one frame.
    ///
    /// `damage` is the window region Masonry reports as changed since the previous frame, in
    /// logical pixels, or `None` to repaint everything. `host_invalidated` presents the whole
    /// frame even if only part of it was re-rasterised, for redraws the window system asked for or
    /// may have asked for; it stays in effect until a full present succeeds.
    pub(crate) fn present(
        &mut self,
        window: &WindowHandle,
        frame: PreparedFrame<'_>,
        damage: Option<&[Rect]>,
        host_invalidated: bool,
    ) {
        self.full_present_pending |= host_invalidated;
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
        match self.present_framebuffer(window, (width, height), updated.as_deref()) {
            Ok(Presentation::Full) => self.full_present_pending = false,
            Ok(_) => {}
            Err(err) => {
                tracing::error!("Couldn't present the softbuffer frame: {err}");
                // The window may now show something other than the framebuffer.
                self.renderer.invalidate();
                self.full_present_pending = true;
            }
        }
    }

    /// Copies the framebuffer into a softbuffer buffer and presents it, returning how.
    fn present_framebuffer<'a>(
        &mut self,
        window: &WindowHandle,
        (width, height): (NonZeroU32, NonZeroU32),
        updated: Option<&'a [PixelRect]>,
    ) -> Result<Presentation<'a>, softbuffer::SoftBufferError> {
        if self.size != Some((width, height)) {
            self.surface.resize(width, height)?;
            self.size = Some((width, height));
        }
        let presentation = presentation(updated, self.full_present_pending, self.retains_contents);
        if presentation == Presentation::Skip {
            return Ok(presentation);
        }
        let mut buffer = self.surface.buffer_mut()?;
        let framebuffer = self.renderer.framebuffer();
        match updated {
            Some(rects) if copy_only_updated(updated, buffer.age()) => {
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
        match presentation {
            Presentation::Damage(rects) => {
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
                buffer.present_with_damage(&damage)?;
            }
            _ => buffer.present()?,
        }
        Ok(presentation)
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

#[cfg(test)]
mod tests {
    use masonry_imaging::PixelRect;

    use super::{Presentation, copy_only_updated, presentation};

    const LINE: [PixelRect; 1] = [PixelRect::new(256, 204, 944, 48)];

    #[test]
    fn damage_is_presented_only_where_contents_are_retained() {
        assert_eq!(
            presentation(Some(&LINE), false, true),
            Presentation::Damage(&LINE)
        );
        assert_eq!(presentation(Some(&LINE), false, false), Presentation::Full);
        assert_eq!(presentation(None, false, true), Presentation::Full);
    }

    #[test]
    fn host_redraws_present_the_whole_frame() {
        // A redraw the window system asked for, alone or merged with a Masonry update.
        assert_eq!(presentation(Some(&LINE), true, true), Presentation::Full);
        assert_eq!(presentation(Some(&[]), true, true), Presentation::Full);
        assert_eq!(presentation(Some(&[]), true, false), Presentation::Full);
    }

    #[test]
    fn unchanged_frames_are_skipped_only_when_retained() {
        assert_eq!(presentation(Some(&[]), false, true), Presentation::Skip);
        // Without retained contents an expose may be hiding behind a Masonry request.
        assert_eq!(presentation(Some(&[]), false, false), Presentation::Full);
    }

    #[test]
    fn only_the_previous_buffer_receives_partial_copies() {
        assert!(copy_only_updated(Some(&LINE), 1));
        assert!(!copy_only_updated(Some(&LINE), 0));
        assert!(!copy_only_updated(Some(&LINE), 2));
        assert!(!copy_only_updated(None, 1));
    }
}
