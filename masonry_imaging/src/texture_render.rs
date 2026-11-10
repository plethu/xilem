// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

//! Host-neutral texture rendering helpers.
//!
//! This module owns backend state for rendering Masonry paint output into a caller-provided WGPU
//! texture target. It does not own window surfaces or presentation.

use wgpu;

use crate::PreparedFrame;

/// GPU target that Masonry content should be rendered into.
#[derive(Clone, Copy, Debug)]
pub struct RenderTarget<'a> {
    /// Adapter used to create the device.
    pub adapter: &'a wgpu::Adapter,
    /// Device used for rendering commands.
    pub device: &'a wgpu::Device,
    /// Queue used for submitting rendering commands and uploads.
    pub queue: &'a wgpu::Queue,
    /// Texture backing the render target.
    pub texture: &'a wgpu::Texture,
    /// View of the render target texture.
    pub view: &'a wgpu::TextureView,
}

/// Errors that can occur while rendering Masonry content into a target texture.
#[derive(Debug)]
pub struct Error(imp::Error);

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Error {}

/// Backend-selected texture renderer for Masonry paint output.
#[derive(Debug)]
pub struct Renderer(imp::Renderer);

impl Renderer {
    /// Stable backend name for diagnostics.
    pub const BACKEND_NAME: &str = imp::Renderer::BACKEND_NAME;

    /// Create an empty renderer state.
    pub fn new() -> Self {
        Self(imp::Renderer::new())
    }

    /// The backend rendering frames.
    ///
    /// With runtime fallback this names the backend chosen for the current adapter, and
    /// [`Self::BACKEND_NAME`] until the first frame selects one.
    pub fn backend_name(&self) -> &'static str {
        #[cfg(feature = "vello_cpu_fallback")]
        return self.0.backend_name();
        #[cfg(not(feature = "vello_cpu_fallback"))]
        Self::BACKEND_NAME
    }

    /// Render the given Masonry paint output into the provided target texture.
    pub fn render_to_texture(
        &mut self,
        target: RenderTarget<'_>,
        frame: PreparedFrame<'_>,
    ) -> Result<(), Error> {
        self.0.render_to_texture(target, frame).map_err(Error)
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(
    all(feature = "imaging_skia", not(target_arch = "wasm32")),
    feature = "imaging_vello",
    feature = "imaging_vello_hybrid"
))]
mod shared_texture_renderer {
    use imaging_wgpu::{TextureRenderer, TextureRendererError};

    use crate::PreparedFrame;

    #[derive(Debug)]
    pub(super) struct RendererCache<R, K> {
        key: Option<K>,
        inner: Option<R>,
    }

    impl<R, K> RendererCache<R, K>
    where
        K: Copy + Eq,
    {
        pub(super) fn new() -> Self {
            Self {
                key: None,
                inner: None,
            }
        }

        pub(super) fn get_or_try_init<E>(
            &mut self,
            key: K,
            create: impl FnOnce() -> Result<R, E>,
        ) -> Result<&mut R, E> {
            if self.key != Some(key) {
                self.inner = Some(create()?);
                self.key = Some(key);
            }

            Ok(self
                .inner
                .as_mut()
                .expect("cached renderer should be initialized"))
        }
    }

    #[cfg(any(feature = "imaging_vello", feature = "imaging_vello_hybrid"))]
    #[inline]
    pub(super) fn device_queue_key(target: super::RenderTarget<'_>) -> (usize, usize) {
        (
            target.device as *const _ as usize,
            target.queue as *const _ as usize,
        )
    }

    pub(super) fn render_window_source_to_texture<R>(
        renderer: &mut R,
        frame: PreparedFrame<'_>,
        target: R::TextureTarget,
    ) -> Result<(), TextureRendererError>
    where
        R: TextureRenderer,
    {
        let mut frame = frame;
        renderer.render_source_into_texture(&mut frame, target)
    }
}

#[cfg(not(any(
    feature = "imaging_vello",
    feature = "imaging_vello_hybrid",
    feature = "imaging_vello_cpu",
    all(feature = "imaging_skia", not(target_arch = "wasm32"))
)))]
mod imp {
    use crate::PreparedFrame;

    #[derive(Debug)]
    pub(super) enum Error {}

    impl core::fmt::Display for Error {
        fn fmt(&self, _: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match *self {}
        }
    }

    impl std::error::Error for Error {}

    #[derive(Debug)]
    pub(super) struct Renderer;

    impl Renderer {
        pub(super) const BACKEND_NAME: &str = "";

        pub(super) fn new() -> Self {
            Self
        }

        pub(super) fn render_to_texture(
            &mut self,
            _: super::RenderTarget<'_>,
            _: PreparedFrame<'_>,
        ) -> Result<(), Error> {
            unreachable!("a renderer backend feature is required")
        }
    }
}

#[cfg(all(
    not(feature = "imaging_vello"),
    feature = "imaging_skia",
    not(target_arch = "wasm32")
))]
mod imp {
    use super::shared_texture_renderer::{RendererCache, render_window_source_to_texture};
    use crate::skia::{TargetRenderer, new_target_renderer};

    use super::RenderTarget;
    use crate::PreparedFrame;
    use imaging_wgpu::TextureRendererError;

    /// Errors that can occur while rendering Masonry content with Skia.
    #[derive(Debug)]
    pub(super) enum Error {
        /// Creating the Skia renderer failed.
        CreateRenderer(crate::skia::Error),
        /// Rendering the Masonry source failed.
        Render(TextureRendererError),
    }

    impl core::fmt::Display for Error {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::CreateRenderer(err) => write!(f, "creating Skia renderer failed: {err}"),
                Self::Render(err) => write!(f, "rendering with Skia failed: {err:?}"),
            }
        }
    }

    impl std::error::Error for Error {}

    /// Runtime renderer state for the Skia backend.
    #[derive(Debug)]
    pub(super) struct Renderer {
        inner: RendererCache<TargetRenderer, (usize, usize, usize)>,
    }

    impl Renderer {
        /// Stable backend name for diagnostics.
        pub(super) const BACKEND_NAME: &str = crate::skia::BACKEND_NAME;

        /// Create an empty renderer state.
        pub(super) fn new() -> Self {
            Self {
                inner: RendererCache::new(),
            }
        }

        /// Render the given Masonry paint output into the provided target texture.
        pub(super) fn render_to_texture(
            &mut self,
            target: RenderTarget<'_>,
            frame: PreparedFrame<'_>,
        ) -> Result<(), Error> {
            let renderer = self.inner.get_or_try_init(
                (
                    target.adapter as *const _ as usize,
                    target.device as *const _ as usize,
                    target.queue as *const _ as usize,
                ),
                || {
                    new_target_renderer(
                        target.adapter.clone(),
                        target.device.clone(),
                        target.queue.clone(),
                    )
                    .map_err(Error::CreateRenderer)
                },
            )?;

            render_window_source_to_texture(renderer, frame, target.texture.clone())
                .map_err(Error::Render)
        }
    }
}

/// Vello CPU rasterises on the host and uploads the frame into the target texture, so the GPU
/// (or a software Vulkan implementation) only copies and presents it.
#[cfg(any(
    feature = "vello_cpu_fallback",
    all(
        feature = "imaging_vello_cpu",
        not(feature = "imaging_vello"),
        not(feature = "imaging_vello_hybrid"),
        not(all(feature = "imaging_skia", not(target_arch = "wasm32")))
    )
))]
mod vello_cpu_texture {
    use imaging::render::{
        ImageBufferFormat, ImageBufferTarget, ImageRenderer, ImageRendererError,
    };

    use super::RenderTarget;
    use crate::PreparedFrame;

    /// Errors that can occur while rendering Masonry content with Vello CPU.
    #[derive(Debug)]
    pub(crate) enum Error {
        /// Rasterising the Masonry source failed.
        Render(ImageRendererError),
    }

    impl core::fmt::Display for Error {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::Render(err) => write!(f, "rendering with Vello CPU failed: {err:?}"),
            }
        }
    }

    impl std::error::Error for Error {}

    /// Runtime renderer state for the Vello CPU backend.
    #[derive(Debug)]
    pub(crate) struct Renderer {
        inner: imaging_vello_cpu::VelloCpuRenderer,
        /// Reused frame pixels, tightly packed RGBA8.
        pixels: Vec<u8>,
    }

    impl Renderer {
        /// Stable backend name for diagnostics.
        pub(crate) const BACKEND_NAME: &str = crate::vello_cpu::BACKEND_NAME;

        /// Create an empty renderer state.
        pub(crate) fn new() -> Self {
            Self {
                inner: imaging_vello_cpu::VelloCpuRenderer::new(1, 1),
                pixels: Vec::new(),
            }
        }

        /// Render the given Masonry paint output into the provided target texture.
        pub(crate) fn render_to_texture(
            &mut self,
            target: RenderTarget<'_>,
            frame: PreparedFrame<'_>,
        ) -> Result<(), Error> {
            let size = target.texture.size();
            let bytes_per_row = size.width as usize * 4;
            self.pixels.resize(bytes_per_row * size.height as usize, 0);
            let mut frame = frame;
            self.inner
                .render_source_into(
                    &mut frame,
                    ImageBufferTarget {
                        data: &mut self.pixels,
                        width: size.width,
                        height: size.height,
                        bytes_per_row,
                        format: ImageBufferFormat::Rgba8Unorm,
                    },
                )
                .map_err(Error::Render)?;
            target.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: target.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size.width * 4),
                    rows_per_image: Some(size.height),
                },
                size,
            );
            Ok(())
        }
    }
}

#[cfg(feature = "imaging_vello")]
mod vello_gpu_texture {
    use super::shared_texture_renderer::{
        RendererCache, device_queue_key, render_window_source_to_texture,
    };
    use crate::vello::{TargetRenderer, TextureTarget, new_target_renderer};

    use super::RenderTarget;
    use crate::PreparedFrame;
    use imaging_wgpu::TextureRendererError;

    /// Errors that can occur while rendering Masonry content with Vello.
    #[derive(Debug)]
    pub(crate) enum Error {
        /// Creating the Vello renderer failed.
        CreateRenderer(crate::vello::Error),
        /// Rendering the scene to the texture failed.
        Render(TextureRendererError),
    }

    impl core::fmt::Display for Error {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::CreateRenderer(err) => write!(f, "creating Vello renderer failed: {err}"),
                Self::Render(err) => write!(f, "rendering with Vello failed: {err:?}"),
            }
        }
    }

    impl std::error::Error for Error {}

    /// Runtime renderer state for the Vello backend.
    #[derive(Debug)]
    pub(crate) struct Renderer {
        inner: RendererCache<TargetRenderer, (usize, usize)>,
    }

    impl Renderer {
        /// Stable backend name for diagnostics.
        pub(crate) const BACKEND_NAME: &str = crate::vello::BACKEND_NAME;

        /// Create an empty renderer state.
        pub(crate) fn new() -> Self {
            Self {
                inner: RendererCache::new(),
            }
        }

        /// Render the given Masonry paint output into the provided target texture.
        pub(crate) fn render_to_texture(
            &mut self,
            target: RenderTarget<'_>,
            frame: PreparedFrame<'_>,
        ) -> Result<(), Error> {
            let renderer = self.inner.get_or_try_init(device_queue_key(target), || {
                new_target_renderer(target.device.clone(), target.queue.clone())
                    .map_err(Error::CreateRenderer)
            })?;

            render_window_source_to_texture(
                renderer,
                frame,
                TextureTarget::new(target.view, frame.width, frame.height),
            )
            .map_err(Error::Render)
        }
    }
}

#[cfg(all(
    not(feature = "imaging_vello"),
    not(all(feature = "imaging_skia", not(target_arch = "wasm32"))),
    feature = "imaging_vello_hybrid"
))]
mod imp {
    use super::shared_texture_renderer::{
        RendererCache, device_queue_key, render_window_source_to_texture,
    };
    use crate::vello_hybrid::{TargetRenderer, TextureTarget, new_target_renderer};

    use super::RenderTarget;
    use crate::PreparedFrame;
    use imaging_wgpu::TextureRendererError;

    /// Errors that can occur while rendering Masonry content with Vello Hybrid.
    #[derive(Debug)]
    pub(super) enum Error {
        /// Rendering the Masonry source failed.
        Render(TextureRendererError),
    }

    impl core::fmt::Display for Error {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::Render(err) => write!(f, "rendering with Vello Hybrid failed: {err:?}"),
            }
        }
    }

    impl std::error::Error for Error {}

    /// Runtime renderer state for the Vello Hybrid backend.
    #[derive(Debug)]
    pub(super) struct Renderer {
        inner: RendererCache<TargetRenderer, (usize, usize)>,
    }

    impl Renderer {
        /// Stable backend name for diagnostics.
        pub(super) const BACKEND_NAME: &str = crate::vello_hybrid::BACKEND_NAME;

        /// Create an empty renderer state.
        pub(super) fn new() -> Self {
            Self {
                inner: RendererCache::new(),
            }
        }

        /// Render the given Masonry paint output into the provided target texture.
        pub(super) fn render_to_texture(
            &mut self,
            target: RenderTarget<'_>,
            frame: PreparedFrame<'_>,
        ) -> Result<(), Error> {
            let renderer = self.inner.get_or_try_init(device_queue_key(target), || {
                Ok(new_target_renderer(
                    target.device.clone(),
                    target.queue.clone(),
                ))
            })?;

            render_window_source_to_texture(
                renderer,
                frame,
                TextureTarget::new(target.view, frame.width, frame.height),
            )
            .map_err(Error::Render)
        }
    }
}

#[cfg(all(feature = "imaging_vello", not(feature = "vello_cpu_fallback")))]
use vello_gpu_texture as imp;

#[cfg(all(
    feature = "imaging_vello_cpu",
    not(feature = "imaging_vello"),
    not(feature = "imaging_vello_hybrid"),
    not(all(feature = "imaging_skia", not(target_arch = "wasm32")))
))]
use vello_cpu_texture as imp;

/// GPU Vello on hardware adapters, Vello CPU on software adapters such as llvmpipe or WARP.
///
/// GPU Vello's compute pipeline is far slower when a software Vulkan or D3D implementation
/// emulates it than rasterising directly on the host. `MASONRY_IMAGING_BACKEND` set to
/// `vello` or `vello_cpu` overrides the choice.
#[cfg(feature = "vello_cpu_fallback")]
mod imp {
    use super::RenderTarget;
    use super::{vello_cpu_texture as cpu, vello_gpu_texture as gpu};
    use crate::PreparedFrame;

    /// Environment variable that forces a backend.
    const OVERRIDE: &str = "MASONRY_IMAGING_BACKEND";

    /// Errors from whichever backend rendered the frame.
    #[derive(Debug)]
    pub(super) enum Error {
        /// GPU Vello failed.
        Gpu(gpu::Error),
        /// Vello CPU failed.
        Cpu(cpu::Error),
    }

    impl core::fmt::Display for Error {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::Gpu(err) => err.fmt(f),
                Self::Cpu(err) => err.fmt(f),
            }
        }
    }

    impl std::error::Error for Error {}

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Backend {
        Gpu,
        Cpu,
    }

    /// Runtime renderer state that chooses a backend per adapter.
    #[derive(Debug)]
    pub(super) struct Renderer {
        gpu: gpu::Renderer,
        cpu: cpu::Renderer,
        /// Adapter identity and the backend chosen for it.
        selected: Option<(usize, Backend)>,
    }

    impl Renderer {
        /// Backend name reported before the first frame selects one.
        pub(super) const BACKEND_NAME: &str = "imaging_vello+imaging_vello_cpu";

        /// Create an empty renderer state.
        pub(super) fn new() -> Self {
            Self {
                gpu: gpu::Renderer::new(),
                cpu: cpu::Renderer::new(),
                selected: None,
            }
        }

        /// The backend rendering frames, once one has been selected.
        pub(super) fn backend_name(&self) -> &'static str {
            match self.selected {
                Some((_, Backend::Gpu)) => gpu::Renderer::BACKEND_NAME,
                Some((_, Backend::Cpu)) => cpu::Renderer::BACKEND_NAME,
                None => Self::BACKEND_NAME,
            }
        }

        /// Render the given Masonry paint output into the provided target texture.
        pub(super) fn render_to_texture(
            &mut self,
            target: RenderTarget<'_>,
            frame: PreparedFrame<'_>,
        ) -> Result<(), Error> {
            let key = target.adapter as *const _ as usize;
            let backend = match self.selected {
                Some((selected, backend)) if selected == key => backend,
                _ => {
                    let backend = select(target.adapter);
                    self.selected = Some((key, backend));
                    backend
                }
            };
            match backend {
                Backend::Gpu => self
                    .gpu
                    .render_to_texture(target, frame)
                    .map_err(Error::Gpu),
                Backend::Cpu => self
                    .cpu
                    .render_to_texture(target, frame)
                    .map_err(Error::Cpu),
            }
        }
    }

    fn select(adapter: &wgpu::Adapter) -> Backend {
        let info = adapter.get_info();
        let forced = std::env::var(OVERRIDE).ok();
        let (backend, reason) = match forced.as_deref() {
            Some("vello") => (Backend::Gpu, "override"),
            Some("vello_cpu") => (Backend::Cpu, "override"),
            _ if info.device_type == wgpu::DeviceType::Cpu => (Backend::Cpu, "software adapter"),
            _ => (Backend::Gpu, "hardware adapter"),
        };
        if let Some(value) = forced
            .as_deref()
            .filter(|value| !matches!(*value, "vello" | "vello_cpu"))
        {
            tracing::warn!("Ignoring {OVERRIDE}={value}; expected `vello` or `vello_cpu`");
        }
        tracing::info!(
            backend = match backend {
                Backend::Gpu => gpu::Renderer::BACKEND_NAME,
                Backend::Cpu => cpu::Renderer::BACKEND_NAME,
            },
            reason,
            adapter = %info.name,
            device_type = ?info.device_type,
            wgpu_backend = ?info.backend,
            "Selected Masonry imaging backend"
        );
        backend
    }
}
