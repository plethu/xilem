// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use core::fmt;

/// Errors that can occur while creating or using a Vello CPU renderer.
#[derive(Debug)]
pub enum Error {
    /// The underlying `imaging_vello_cpu` renderer failed.
    Backend(imaging_vello_cpu::Error),
    /// Rasterising a prepared frame failed.
    Render(imaging::render::ImageRendererError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(err) => write!(f, "Vello CPU render failed: {err:?}"),
            Self::Render(err) => write!(f, "Vello CPU frame render failed: {err:?}"),
        }
    }
}

impl std::error::Error for Error {}

/// Stable backend name for diagnostics.
pub const BACKEND_NAME: &str = "imaging_vello_cpu";

/// Masonry alias for the selected Vello CPU renderer type.
pub type Renderer = imaging_vello_cpu::VelloCpuRenderer;

/// Create a reusable headless Vello CPU renderer.
pub fn new_headless_renderer() -> Result<Renderer, Error> {
    Ok(imaging_vello_cpu::VelloCpuRenderer::new(1, 1))
}

/// Rasterise a prepared window frame into tightly packed, unpremultiplied RGBA8 pixels.
///
/// `pixels` is reused across frames and resized to the frame's dimensions.
pub fn render_rgba8(
    renderer: &mut Renderer,
    frame: crate::PreparedFrame<'_>,
    pixels: &mut Vec<u8>,
) -> Result<(), Error> {
    use imaging::render::{ImageBufferFormat, ImageBufferTarget, ImageRenderer};

    let (width, height) = (frame.width, frame.height);
    let bytes_per_row = width as usize * 4;
    pixels.resize(bytes_per_row * height as usize, 0);
    let mut frame = frame;
    renderer
        .render_source_into(
            &mut frame,
            ImageBufferTarget {
                data: pixels,
                width,
                height,
                bytes_per_row,
                format: ImageBufferFormat::Rgba8Unorm,
            },
        )
        .map_err(Error::Render)
}
