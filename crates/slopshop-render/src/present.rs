//! Direct presentation to a window surface (ADR 0002).
//!
//! The composited view is copied into the surface's back buffer and presented: no readback and
//! no transfer of pixels to the UI, which is what dominates the cost of the frame path. The
//! caller decides where the view goes in the surface (e.g. the canvas area of a window whose
//! UI is drawn over the rest).

use slopshop_core::view::ViewTransform;
use slopshop_core::{Document, Rect, Size};

use crate::{RenderError, Renderer, cache::FrameOptions};

/// The renderer's packed RGBA8 sRGB pixels are copied into the surface as they are.
const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Buffer-to-texture copies need rows aligned to 256 bytes, i.e. 64 RGBA8 pixels.
const ROW_ALIGNMENT_PIXELS: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT / 4;

/// A window surface that views are presented to.
#[derive(Debug)]
pub struct Presenter {
    surface: wgpu::Surface<'static>,
    alpha_mode: wgpu::CompositeAlphaMode,
    /// Size the surface is configured for; `None` before the first present.
    configured: Option<Size>,
    /// The surface asked to be configured again (it no longer matches the window).
    stale: bool,
}

/// What [`Renderer::present_view`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presented {
    Frame,
    /// Shown, with parts from a coarser level of the display cache while their tiles are
    /// composited (ADR 0022): present again soon to refine them.
    Partial,
    /// Nothing was shown (empty or occluded window, swapchain busy or outdated): present
    /// again on the next change.
    Skipped,
}

impl Renderer {
    /// A presenter for a window: anything that provides window and display handles, such as a
    /// Tauri window.
    pub fn create_presenter(
        &self,
        window: impl Into<wgpu::SurfaceTarget<'static>>,
    ) -> Result<Presenter, RenderError> {
        let surface = self
            .instance
            .create_surface(window)
            .map_err(|e| RenderError::Surface(e.to_string()))?;
        let caps = surface.get_capabilities(&self.adapter);
        if !caps.formats.contains(&SURFACE_FORMAT) {
            return Err(RenderError::Surface(format!(
                "{SURFACE_FORMAT:?} is not supported by this surface (supported: {:?})",
                caps.formats
            )));
        }
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST;
        if !caps.usages.contains(usage) {
            return Err(RenderError::Surface(format!(
                "the surface does not allow {usage:?} (allowed: {:?})",
                caps.usages
            )));
        }
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            wgpu::CompositeAlphaMode::Auto
        };
        Ok(Presenter {
            surface,
            alpha_mode,
            configured: None,
            stale: false,
        })
    }

    /// Present `view` of `document` in `rect` of the surface (surface pixels), resizing the
    /// surface to `surface_size` first if needed. The rest of the surface is cleared to `clear`
    /// (sRGB-encoded RGBA); the part of `rect` outside the surface is not drawn. Blocks until a
    /// back buffer is available (at most about one display refresh), never on a readback.
    /// Progressive: when composited tiles are missing, only some are composited and the rest is
    /// shown from a coarser level ([`Presented::Partial`]).
    #[allow(clippy::too_many_arguments)]
    pub fn present_view(
        &self,
        presenter: &mut Presenter,
        document: &Document,
        view: ViewTransform,
        overlays: crate::ViewOverlays,
        rect: Rect,
        surface_size: Size,
        clear: [f64; 4],
    ) -> Result<Presented, RenderError> {
        if surface_size.is_empty() {
            return Ok(Presented::Skipped);
        }
        if presenter.stale || presenter.configured != Some(surface_size) {
            self.configure(presenter, surface_size)?;
        }
        let frame = match presenter.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                presenter.stale = true;
                frame
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                presenter.stale = true;
                return Ok(Presented::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(Presented::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                return Err(RenderError::Surface("surface lost".into()));
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RenderError::Surface(
                    "validation error while acquiring a frame".into(),
                ));
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let visible = clip(rect, surface_size);
        let covers_surface =
            visible == Some(Rect::new(0, 0, surface_size.width, surface_size.height));
        let clear_color = wgpu::Color {
            r: clear[0],
            g: clear[1],
            b: clear[2],
            a: clear[3],
        };
        let clear_pass = |encoder: &mut wgpu::CommandEncoder| {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        };

        let mut presented = Presented::Frame;
        match visible {
            Some(visible) => {
                let output = padded_output(visible);
                let options = FrameOptions {
                    timestamps: None,
                    progressive: true,
                };
                let stats = self.composite(
                    document,
                    view,
                    overlays,
                    output,
                    options,
                    |encoder, pixels| {
                        if !covers_surface {
                            clear_pass(encoder);
                        }
                        // Only the visible columns are copied; padding columns are dropped.
                        encoder.copy_buffer_to_texture(
                            wgpu::TexelCopyBufferInfo {
                                buffer: pixels,
                                layout: wgpu::TexelCopyBufferLayout {
                                    offset: 0,
                                    bytes_per_row: Some(output.width * 4),
                                    rows_per_image: Some(output.height),
                                },
                            },
                            wgpu::TexelCopyTextureInfo {
                                texture: &frame.texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: visible.x,
                                    y: visible.y,
                                    z: 0,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::Extent3d {
                                width: visible.width,
                                height: visible.height,
                                depth_or_array_layers: 1,
                            },
                        );
                    },
                )?;
                if stats.incomplete {
                    presented = Presented::Partial;
                }
            }
            None => {
                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("present clear"),
                        });
                clear_pass(&mut encoder);
                self.queue.submit([encoder.finish()]);
            }
        }
        self.queue.present(frame);
        Ok(presented)
    }

    /// Configure the surface for `size`. A size the device cannot handle (e.g. beyond its
    /// texture size limit) is an error, not a panic, and leaves the surface unconfigured.
    fn configure(&self, presenter: &mut Presenter, size: Size) -> Result<(), RenderError> {
        let max = self.device.limits().max_texture_dimension_2d;
        if size.width > max || size.height > max {
            return Err(RenderError::Surface(format!(
                "surface size {}×{} exceeds the device's texture size limit ({max})",
                size.width, size.height
            )));
        }
        self.capture_errors(|| {
            presenter.surface.configure(
                &self.device,
                &wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
                    format: SURFACE_FORMAT,
                    color_space: wgpu::SurfaceColorSpace::Auto,
                    width: size.width,
                    height: size.height,
                    present_mode: wgpu::PresentMode::AutoVsync,
                    // One frame queued at most: input-to-screen latency over throughput.
                    desired_maximum_frame_latency: 1,
                    alpha_mode: presenter.alpha_mode,
                    view_formats: Vec::new(),
                },
            );
            Ok(())
        })?;
        presenter.configured = Some(size);
        presenter.stale = false;
        Ok(())
    }
}

/// The part of `rect` inside a `surface`-sized area, if any.
fn clip(rect: Rect, surface: Size) -> Option<Rect> {
    if rect.x >= surface.width || rect.y >= surface.height {
        return None;
    }
    let width = rect.width.min(surface.width - rect.x);
    let height = rect.height.min(surface.height - rect.y);
    (width > 0 && height > 0).then(|| Rect::new(rect.x, rect.y, width, height))
}

/// Output size rendered for `visible`: whole 256-byte rows, as buffer-to-texture copies need.
/// The extra columns on the right render more of the document and are never copied.
fn padded_output(visible: Rect) -> Size {
    Size::new(
        visible.width.next_multiple_of(ROW_ALIGNMENT_PIXELS),
        visible.height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rects_are_clipped_to_the_surface() {
        let surface = Size::new(800, 600);
        assert_eq!(
            clip(Rect::new(0, 56, 800, 600), surface),
            Some(Rect::new(0, 56, 800, 544))
        );
        assert_eq!(
            clip(Rect::new(10, 20, 30, 40), surface),
            Some(Rect::new(10, 20, 30, 40))
        );
        assert_eq!(clip(Rect::new(800, 0, 10, 10), surface), None);
        assert_eq!(clip(Rect::new(0, 0, 0, 10), surface), None);
    }

    #[test]
    fn outputs_have_aligned_rows() {
        assert_eq!(padded_output(Rect::new(0, 0, 1, 7)), Size::new(64, 7));
        assert_eq!(padded_output(Rect::new(5, 0, 64, 1)), Size::new(64, 1));
        assert_eq!(
            padded_output(Rect::new(0, 0, 2561, 1440)),
            Size::new(2624, 1440)
        );
        assert_eq!(ROW_ALIGNMENT_PIXELS * 4, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    }
}
