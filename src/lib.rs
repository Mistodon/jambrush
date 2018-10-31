extern crate gfx_backend_metal as backend;
extern crate gfx_hal;
extern crate image;
extern crate rusttype;
extern crate texture_packer;
extern crate winit;

mod gfxutils;

use std::path::Path;

use image::{DynamicImage, Rgba, RgbaImage};
use rusttype::{Font as RTFont, gpu_cache::Cache as RTCache, PositionedGlyph};
use texture_packer::TexturePacker;
use winit::Window;

use gfxutils::*;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct SpritePushConstants {
    pub transform: [[f32; 4]; 4],
    pub tint: [f32; 4],
    pub uv_origin: [f32; 2],
    pub uv_scale: [f32; 2],
}

#[derive(Debug)]
pub struct Sprite {
    id: usize,
}

#[derive(Debug)]
pub struct Font {
    id: usize,
}

// TODO: all the unwraps...

// TODO: Lots. Think about resolution/rebuilding RTT texture
pub struct JamBrushSystem {
    instance: backend::Instance,
    surface: backend::Surface,
    adapter: gfx_hal::Adapter<backend::Backend>,
    device: backend::Device,
    queue_group: gfx_hal::QueueGroup<backend::Backend, Graphics>,
    command_pool: gfx_hal::CommandPool<backend::Backend, Graphics>,
    surface_color_format: Format,
    render_pass: TRenderPass,
    set_layout: TDescriptorSetLayout,
    pipeline_layout: TPipelineLayout,
    pipeline: TGraphicsPipeline,
    desc_pool: TDescriptorPool,
    sprites_desc_set: TDescriptorSet,
    blit_desc_set: TDescriptorSet,
    texture_semaphore: TSemaphore,
    scene_semaphore: TSemaphore,
    frame_semaphore: TSemaphore,
    present_semaphore: TSemaphore,
    rtt_image: TImage,
    rtt_memory: TMemory,
    rtt_view: TImageView,
    rtt_sampler: TSampler,
    rtt_framebuffer: TFramebuffer,
    texture_fence: TFence,
    sprite_textures: Vec<RgbaImage>,
    sprite_regions: Vec<([f32; 2], [f32; 2], [f32; 2])>,
    atlas_image: RgbaImage,
    atlas_texture: TImage,
    atlas_memory: TMemory,
    atlas_view: TImageView,
    atlas_sampler: TSampler,
    fonts: Vec<RTFont<'static>>,
    glyph_cache: RTCache<'static>,
    swapchain: Option<(
        backend::Swapchain,
        Extent,
        Vec<TImage>,
        Vec<TImageView>,
        Vec<TFramebuffer>,
    )>,
    swapchain_invalidated: bool,
    resolution: (u32, u32),
    dpi_factor: f64,
    _private: (),
}

impl JamBrushSystem {
    pub fn new(window: &Window, resolution: (u32, u32)) -> Self {
        let instance = backend::Instance::create("JamBrush", 1);
        let mut surface = instance.create_surface(&window);
        let mut adapter = instance.enumerate_adapters().remove(0);
        let (device, mut queue_group) = adapter
            .open_with::<_, Graphics>(1, |family| surface.supports_queue_family(family))
            .unwrap();

        let mut command_pool = device
            .create_command_pool_typed(&queue_group, CommandPoolCreateFlags::empty(), 16)
            .unwrap();

        let (_caps, formats, _) = surface.compatibility(&adapter.physical_device);

        let surface_color_format = {
            match formats {
                Some(choices) => choices
                    .into_iter()
                    .find(|format| format.base_format().1 == ChannelType::Srgb)
                    .unwrap(),
                None => Format::Rgba8Srgb,
            }
        };

        let render_pass = {
            let color_attachment = Attachment {
                format: Some(surface_color_format),
                samples: 1,
                ops: AttachmentOps::new(AttachmentLoadOp::Clear, AttachmentStoreOp::Store),
                stencil_ops: AttachmentOps::DONT_CARE,
                layouts: Layout::Undefined..Layout::Present,
            };

            let subpass = SubpassDesc {
                colors: &[(0, Layout::ColorAttachmentOptimal)],
                depth_stencil: None,
                inputs: &[],
                resolves: &[],
                preserves: &[],
            };

            let dependency = SubpassDependency {
                passes: SubpassRef::External..SubpassRef::Pass(0),
                stages: PipelineStage::COLOR_ATTACHMENT_OUTPUT
                    ..PipelineStage::COLOR_ATTACHMENT_OUTPUT,
                accesses: Access::empty()
                    ..(Access::COLOR_ATTACHMENT_READ | Access::COLOR_ATTACHMENT_WRITE),
            };

            device
                .create_render_pass(&[color_attachment], &[subpass], &[dependency])
                .unwrap()
        };

        let set_layout = device
            .create_descriptor_set_layout(
                &[
                    DescriptorSetLayoutBinding {
                        binding: 0,
                        ty: DescriptorType::SampledImage,
                        count: 1,
                        stage_flags: ShaderStageFlags::FRAGMENT,
                        immutable_samplers: false,
                    },
                    DescriptorSetLayoutBinding {
                        binding: 1,
                        ty: DescriptorType::Sampler,
                        count: 1,
                        stage_flags: ShaderStageFlags::FRAGMENT,
                        immutable_samplers: false,
                    },
                ],
                &[],
            ).unwrap();

        let push_size = utils::push_constant_size::<SpritePushConstants>() as u32;
        let pipeline_layout = device
            .create_pipeline_layout(
                vec![&set_layout],
                &[(ShaderStageFlags::VERTEX, 0..push_size)],
            ).unwrap();

        let vertex_shader_module = {
            let spirv = include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/compiled/sprite.vert.spv"
            ));
            device.create_shader_module(spirv).unwrap()
        };

        let fragment_shader_module = {
            let spirv = include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/compiled/sprite.frag.spv"
            ));
            device.create_shader_module(spirv).unwrap()
        };

        let pipeline = {
            let vs_entry = EntryPoint::<backend::Backend> {
                entry: "main",
                module: &vertex_shader_module,
                specialization: Default::default(),
            };

            let fs_entry = EntryPoint::<backend::Backend> {
                entry: "main",
                module: &fragment_shader_module,
                specialization: Default::default(),
            };

            let shader_entries = GraphicsShaderSet {
                vertex: vs_entry,
                hull: None,
                domain: None,
                geometry: None,
                fragment: Some(fs_entry),
            };

            let subpass = Subpass {
                index: 0,
                main_pass: &render_pass,
            };

            let mut pipeline_desc = GraphicsPipelineDesc::new(
                shader_entries,
                Primitive::TriangleList,
                Rasterizer::FILL,
                &pipeline_layout,
                subpass,
            );

            pipeline_desc
                .blender
                .targets
                .push(ColorBlendDesc(ColorMask::ALL, BlendState::ALPHA));

            device
                .create_graphics_pipeline(&pipeline_desc, None)
                .expect("create_graphics_pipeline failed")
        };

        let mut desc_pool = device
            .create_descriptor_pool(
                2,
                &[
                    DescriptorRangeDesc {
                        ty: DescriptorType::SampledImage,
                        count: 2,
                    },
                    DescriptorRangeDesc {
                        ty: DescriptorType::Sampler,
                        count: 2,
                    },
                ],
            ).unwrap();

        let sprites_desc_set = desc_pool.allocate_set(&set_layout).unwrap();
        let blit_desc_set = desc_pool.allocate_set(&set_layout).unwrap();

        let texture_semaphore = device.create_semaphore().unwrap();
        let scene_semaphore = device.create_semaphore().unwrap();
        let frame_semaphore = device.create_semaphore().unwrap();
        let present_semaphore = device.create_semaphore().unwrap();

        let memory_types = adapter.physical_device.memory_properties().memory_types;

        let (rtt_image, rtt_memory, rtt_view, rtt_sampler, rtt_framebuffer) = {
            let extent = Extent {
                width: resolution.0,
                height: resolution.1,
                depth: 1,
            };

            let (rtt_image, rtt_memory, rtt_view) = utils::create_image(
                &device,
                &memory_types,
                extent.width,
                extent.height,
                Format::Rgba8Srgb,
                img::Usage::SAMPLED,
                Aspects::COLOR,
            );

            let rtt_sampler = device
                .create_sampler(img::SamplerInfo::new(Filter::Nearest, WrapMode::Clamp))
                .unwrap();

            let rtt_framebuffer = device
                .create_framebuffer(&render_pass, vec![&rtt_view], extent)
                .unwrap();

            (
                rtt_image,
                rtt_memory,
                rtt_view,
                rtt_sampler,
                rtt_framebuffer,
            )
        };

        let limits = adapter.physical_device.limits();
        let atlas_size = limits.max_texture_size as u32;

        let atlas_image = DynamicImage::new_rgba8(atlas_size, atlas_size).to_rgba();

        let texture_fence = device.create_fence(false).unwrap();

        let (atlas_texture, atlas_memory, atlas_view, atlas_sampler) = {
            let (texture_image, texture_memory, texture_view) = utils::create_image(
                &device,
                &memory_types,
                atlas_size,
                atlas_size,
                Format::Rgba8Srgb,
                img::Usage::TRANSFER_DST | img::Usage::SAMPLED,
                Aspects::COLOR,
            );

            let texture_sampler = device
                .create_sampler(img::SamplerInfo::new(Filter::Nearest, WrapMode::Clamp))
                .unwrap();

            // TODO: Maybe allow pre-loading sprites here?

            (texture_image, texture_memory, texture_view, texture_sampler)
        };

        let glyph_cache = RTCache::builder()
            .dimensions(atlas_size, atlas_size / 2)
            .position_tolerance(0.1)
            .scale_tolerance(0.1)
            .pad_glyphs(true)
            .multithread(true)
            .build();

        device.write_descriptor_sets(vec![
            DescriptorSetWrite {
                set: &blit_desc_set,
                binding: 0,
                array_offset: 0,
                descriptors: Some(Descriptor::Image(&rtt_view, Layout::Undefined)),
            },
            DescriptorSetWrite {
                set: &blit_desc_set,
                binding: 1,
                array_offset: 0,
                descriptors: Some(Descriptor::Sampler(&rtt_sampler)),
            },
        ]);

        device.write_descriptor_sets(vec![
            DescriptorSetWrite {
                set: &sprites_desc_set,
                binding: 0,
                array_offset: 0,
                descriptors: Some(Descriptor::Image(&atlas_view, Layout::Undefined)),
            },
            DescriptorSetWrite {
                set: &sprites_desc_set,
                binding: 1,
                array_offset: 0,
                descriptors: Some(Descriptor::Sampler(&atlas_sampler)),
            },
        ]);

        let swapchain = None;

        JamBrushSystem {
            instance,
            surface,
            adapter,
            device,
            queue_group,
            command_pool,
            surface_color_format,
            render_pass,
            set_layout,
            pipeline_layout,
            pipeline,
            desc_pool,
            sprites_desc_set,
            blit_desc_set,
            texture_semaphore,
            scene_semaphore,
            frame_semaphore,
            present_semaphore,
            rtt_image,
            rtt_memory,
            rtt_view,
            rtt_sampler,
            rtt_framebuffer,
            texture_fence,
            sprite_textures: vec![],
            sprite_regions: vec![],
            atlas_image,
            atlas_texture,
            atlas_memory,
            atlas_view,
            atlas_sampler,
            fonts: vec![],
            glyph_cache,
            swapchain,
            swapchain_invalidated: true,
            resolution,
            dpi_factor: window.get_hidpi_factor(),
            _private: (),
        }
    }

    pub fn destroy(mut self) {
        if self.swapchain.is_some() {
            self.destroy_swapchain();
        }

        let JamBrushSystem {
            device,
            command_pool,
            render_pass,
            set_layout,
            pipeline_layout,
            pipeline,
            desc_pool,
            texture_semaphore,
            scene_semaphore,
            frame_semaphore,
            present_semaphore,
            rtt_image,
            rtt_memory,
            rtt_view,
            rtt_sampler,
            rtt_framebuffer,
            texture_fence,
            atlas_texture,
            atlas_memory,
            atlas_view,
            atlas_sampler,
            ..
        } = self;

        device.destroy_sampler(atlas_sampler);
        device.destroy_image_view(atlas_view);
        device.free_memory(atlas_memory);
        device.destroy_image(atlas_texture);
        device.destroy_fence(texture_fence);
        device.destroy_framebuffer(rtt_framebuffer);
        device.destroy_sampler(rtt_sampler);
        device.destroy_image_view(rtt_view);
        device.free_memory(rtt_memory);
        device.destroy_image(rtt_image);
        device.destroy_semaphore(present_semaphore);
        device.destroy_semaphore(frame_semaphore);
        device.destroy_semaphore(scene_semaphore);
        device.destroy_semaphore(texture_semaphore);
        device.destroy_descriptor_pool(desc_pool);
        device.destroy_graphics_pipeline(pipeline);
        device.destroy_pipeline_layout(pipeline_layout);
        device.destroy_descriptor_set_layout(set_layout);
        device.destroy_command_pool(command_pool.into_raw());
        device.destroy_render_pass(render_pass);
    }

    pub fn start_rendering(
        &mut self,
        canvas_clear_color: [f32; 4],
        border_clear_color: Option<[f32; 4]>,
    ) -> JamBrushRenderer {
        JamBrushRenderer::new(
            self,
            canvas_clear_color,
            border_clear_color.unwrap_or(canvas_clear_color),
        )
    }

    pub fn window_resized(&mut self, _resolution: (u32, u32)) {
        self.swapchain_invalidated = true;
    }

    pub fn dpi_factor_changed(&mut self, dpi_factor: f64) {
        self.dpi_factor = dpi_factor;
        self.swapchain_invalidated = true;
    }

    pub fn load_sprite_file<P: AsRef<Path>>(&mut self, path: P) -> Sprite {
        let image_bytes = std::fs::read(path.as_ref()).unwrap();
        let sprite_img =  image::load_from_memory(&image_bytes).unwrap().to_rgba();
        let (w, h) = sprite_img.dimensions();

        self.load_sprite([w, h], &sprite_img)
    }

    pub fn load_sprite(&mut self, size: [u32; 2], data: &[u8]) -> Sprite {
        use image::{GenericImage, ImageBuffer, Rgba};
        use texture_packer::TexturePackerConfig;

        let sprite_index = self.sprite_textures.len();
        let sprite_img: RgbaImage = RgbaImage::from_raw(size[0], size[1], data.to_owned()).unwrap();
        self.sprite_textures.push(sprite_img);

        let (aw, ah) = self.atlas_image.dimensions();
        let atlas_config = TexturePackerConfig {
            max_width: aw,
            max_height: ah / 2,
            allow_rotation: false,
            ..Default::default()
        };

        {
            let mut atlas_packer = TexturePacker::new_skyline(atlas_config);

            for (index, texture) in self.sprite_textures.iter().enumerate() {
                // TODO: ugh, string keys?
                atlas_packer.pack_ref(index.to_string(), texture);
            }

            self.sprite_regions.clear();
            for (i, texture) in self.sprite_textures.iter().enumerate() {
                let frame = atlas_packer.get_frame(&i.to_string()).unwrap().frame;
                let x = frame.x as f32 / aw as f32;
                let y = frame.y as f32 / ah as f32;
                let w = frame.w as f32 / aw as f32;
                let h = frame.h as f32 / ah as f32;
                let (pw, ph) = texture.dimensions();
                self.sprite_regions
                    .push(([x, y], [w, h], [pw as f32, ph as f32]));
                self.atlas_image.copy_from(texture, frame.x, frame.y);
            }
        }

        self.update_atlas();

        Sprite { id: sprite_index }
    }

    pub fn load_font_file<P: AsRef<Path>>(&mut self, path: P) -> Font {
        let font_bytes = std::fs::read(path.as_ref()).unwrap();
        let font = RTFont::from_bytes(font_bytes).unwrap();
        let font_index = self.fonts.len();
        self.fonts.push(font);

        Font { id: font_index }
    }

    pub fn load_font(&mut self, font_bytes: &[u8]) -> Font {
        let font = RTFont::from_bytes(font_bytes.to_owned()).unwrap();
        let font_index = self.fonts.len();
        self.fonts.push(font);

        Font { id: font_index }
    }

    fn update_atlas(&mut self) {
        utils::upload_image_data(
            &self.device,
            &self.adapter.physical_device,
            &mut self.command_pool,
            &mut self.queue_group.queues[0],
            &self.texture_fence,
            &self.atlas_image,
            &self.atlas_texture,
        );
    }

    fn update_swapchain(&mut self) {
        if self.swapchain_invalidated && self.swapchain.is_some() {
            self.destroy_swapchain();
        }

        if self.swapchain.is_none() {
            self.create_swapchain();
        }
    }

    fn destroy_swapchain(&mut self) {
        let (swapchain, _extent, _frame_images, frame_views, framebuffers) =
            self.swapchain.take().unwrap();

        self.device.wait_idle().unwrap();
        self.command_pool.reset();

        for framebuffer in framebuffers {
            self.device.destroy_framebuffer(framebuffer);
        }

        for image_view in frame_views {
            self.device.destroy_image_view(image_view);
        }

        self.device.destroy_swapchain(swapchain);
    }

    fn create_swapchain(&mut self) {
        self.swapchain_invalidated = false;
        let (caps, _, _) = self.surface.compatibility(&self.adapter.physical_device);

        let mut swap_config = SwapchainConfig::from_caps(&caps, self.surface_color_format);
        swap_config.extent.width =
            (f64::from(swap_config.extent.width) * self.dpi_factor).round() as u32;
        swap_config.extent.height =
            (f64::from(swap_config.extent.height) * self.dpi_factor).round() as u32;

        let extent = swap_config.extent.to_extent();
        let (swapchain, backbuffer) = self
            .device
            .create_swapchain(&mut self.surface, swap_config, None)
            .unwrap();

        let (frame_images, frame_views, framebuffers) = match backbuffer {
            Backbuffer::Images(images) => {
                let color_range = SubresourceRange {
                    aspects: Aspects::COLOR,
                    levels: 0..1,
                    layers: 0..1,
                };

                let image_views = images
                    .iter()
                    .map(|image| {
                        self.device
                            .create_image_view(
                                image,
                                ViewKind::D2,
                                self.surface_color_format,
                                Swizzle::NO,
                                color_range.clone(),
                            ).unwrap()
                    }).collect::<Vec<_>>();

                let fbos = image_views
                    .iter()
                    .map(|image_view| {
                        self.device
                            .create_framebuffer(&self.render_pass, vec![image_view], extent)
                            .unwrap()
                    }).collect();

                (images, image_views, fbos)
            }
            Backbuffer::Framebuffer(fbo) => (vec![], vec![], vec![fbo]),
        };

        self.swapchain = Some((swapchain, extent, frame_images, frame_views, framebuffers));
    }
}

pub struct JamBrushRenderer<'a> {
    draw_system: &'a mut JamBrushSystem,
    canvas_clear_color: [f32; 4],
    frame_index: SwapImageIndex,
    blit_command_buffer: Option<Submit<backend::Backend, Graphics, OneShot, Primary>>,
    sprites: Vec<(f32, SpritePushConstants)>,
    glyphs: Vec<(f32, PositionedGlyph<'static>)>,
    finished: bool,
}

impl<'a> JamBrushRenderer<'a> {
    fn new(
        draw_system: &'a mut JamBrushSystem,
        canvas_clear_color: [f32; 4],
        border_clear_color: [f32; 4],
    ) -> Self {
        draw_system.update_swapchain();
        draw_system.command_pool.reset();

        let frame_index: SwapImageIndex;
        let blit_command_buffer: Submit<backend::Backend, Graphics, OneShot, Primary>;

        {
            let (swapchain, extent, frame_images, _frame_views, framebuffers) =
                draw_system.swapchain.as_mut().unwrap();

            // TODO: handle failure
            frame_index = swapchain
                .acquire_image(!0, FrameSync::Semaphore(&draw_system.frame_semaphore))
                .unwrap();

            blit_command_buffer = {
                let mut command_buffer = draw_system.command_pool.acquire_command_buffer(false);

                let (vwidth, vheight) = draw_system.resolution;

                let base_width = (f64::from(vwidth) * draw_system.dpi_factor) as u32;
                let base_height = (f64::from(vheight) * draw_system.dpi_factor) as u32;
                let integer_scale =
                    std::cmp::min(extent.width / base_width, extent.height / base_height);

                let (viewport_width, viewport_height) = if integer_scale == 0 {
                    let viewport_width =
                        std::cmp::min(extent.width, (extent.height * vwidth) / vheight);
                    let viewport_height =
                        std::cmp::min(extent.height, (extent.width * vheight) / vwidth);
                    (viewport_width, viewport_height)
                } else {
                    let viewport_width = base_width * integer_scale;
                    let viewport_height = base_height * integer_scale;
                    (viewport_width, viewport_height)
                };

                let viewport_x = (extent.width - viewport_width) / 2;
                let viewport_y = (extent.height - viewport_height) / 2;

                let viewport = Viewport {
                    rect: Rect {
                        x: viewport_x as i16,
                        y: viewport_y as i16,
                        w: viewport_width as i16,
                        h: viewport_height as i16,
                    },
                    depth: 0.0..1.0,
                };

                command_buffer.set_viewports(0, &[viewport.clone()]);
                command_buffer.set_scissors(0, &[viewport.rect]);

                command_buffer.bind_graphics_pipeline(&draw_system.pipeline);
                command_buffer.bind_graphics_descriptor_sets(
                    &draw_system.pipeline_layout,
                    0,
                    vec![&draw_system.blit_desc_set],
                    &[],
                );

                {
                    let mut encoder = command_buffer.begin_render_pass_inline(
                        &draw_system.render_pass,
                        &framebuffers[frame_index as usize],
                        viewport.rect,
                        &[ClearValue::Color(ClearColor::Float(border_clear_color))],
                    );

                    encoder.push_graphics_constants(
                        &draw_system.pipeline_layout,
                        ShaderStageFlags::VERTEX,
                        0,
                        utils::push_constant_data(&SpritePushConstants {
                            transform: [
                                [2.0, 0.0, 0.0, 0.0],
                                [0.0, 2.0, 0.0, 0.0],
                                [0.0, 0.0, 1.0, 0.0],
                                [-1.0, -1.0, 0.0, 1.0],
                            ],
                            tint: [1.0, 1.0, 1.0, 1.0],
                            uv_origin: [0.0, 0.0],
                            uv_scale: [1.0, 1.0],
                        }),
                    );

                    encoder.draw(0..6, 0..1);
                }

                command_buffer.finish()
            };
        }

        JamBrushRenderer {
            draw_system,
            canvas_clear_color,
            frame_index,
            blit_command_buffer: Some(blit_command_buffer),
            sprites: vec![],
            glyphs: vec![],
            finished: false,
        }
    }

    pub fn sprite(&mut self, sprite: &Sprite, pos: [f32; 2], depth: f32) {
        let (uv_origin, uv_scale, pixel_scale) = self.draw_system.sprite_regions[sprite.id];
        let (res_x, res_y) = self.draw_system.resolution;

        let data = SpritePushConstants {
            transform: make_transform(pos, pixel_scale, [res_x as f32, res_y as f32]),
            tint: [1.0, 1.0, 1.0, 1.0],
            uv_origin,
            uv_scale,
        };

        self.sprites.push((depth, data));
    }

    pub fn text(&mut self, font: &Font, text: &str, pos: [f32; 2], scale: [f32; 2], depth: f32) {
        use rusttype::{Scale, Point};

        // TODO: scale/pos are in pixels - but should be in abstract screen-space units
        // TODO: copyin' a lotta glyphs here!

        let font_id = font.id;
        let font = &self.draw_system.fonts[font_id];
        let glyphs = font.layout(text, Scale { x: scale[0], y: scale[1] }, Point { x: pos[0], y: pos[1] } );

        for glyph in glyphs {
            let glyph = glyph.standalone();
            self.draw_system.glyph_cache.queue_glyph(font_id, glyph.clone());
            self.glyphs.push((depth, glyph));
        }
    }

    fn update_font_atlas(&mut self) {
        {
            let glyph_cache = &mut self.draw_system.glyph_cache;
            let font_atlas_image = &mut self.draw_system.atlas_image;

            let atlas_height = font_atlas_image.height();

            glyph_cache.cache_queued(|dest_rect, data| {
                use image::{GenericImage, ImageBuffer, Rgba};
                use rusttype::Point;

                let Point { x, y } = dest_rect.min;
                let w = dest_rect.width();
                let h = dest_rect.height();

                let mut rgba_buffer = vec![0; data.len() * 4];
                for (&alpha, rgba) in data.into_iter().zip(rgba_buffer.chunks_mut(4)) {
                    rgba[0] = 255;
                    rgba[1] = 255;
                    rgba[2] = 255;
                    rgba[3] = alpha;
                }

                let image_region = RgbaImage::from_raw(w, h, rgba_buffer).unwrap();
                font_atlas_image.copy_from(&image_region, x, y + atlas_height / 2);
            }).unwrap();
        }

        // TODO: Use a separate font texture, in a texture array
        self.draw_system.update_atlas();
    }

    pub fn finish(mut self) {
        self.update_font_atlas();

        let (res_x, res_y) = self.draw_system.resolution;

        let (swapchain, extent, frame_images, _frame_views, framebuffers) =
            self.draw_system.swapchain.as_mut().unwrap();

        let scene_command_buffer = {
            let mut command_buffer = self.draw_system.command_pool.acquire_command_buffer(false);

            let (vwidth, vheight) = self.draw_system.resolution;
            let viewport = Viewport {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: vwidth as i16,
                    h: vheight as i16,
                },
                depth: 0.0..1.0,
            };

            command_buffer.set_viewports(0, &[viewport.clone()]);
            command_buffer.set_scissors(0, &[viewport.rect]);

            command_buffer.bind_graphics_pipeline(&self.draw_system.pipeline);

            {
                let mut encoder = command_buffer.begin_render_pass_inline(
                    &self.draw_system.render_pass,
                    &self.draw_system.rtt_framebuffer,
                    viewport.rect,
                    &[ClearValue::Color(ClearColor::Float(
                        self.canvas_clear_color,
                    ))],
                );

                encoder.bind_graphics_descriptor_sets(
                    &self.draw_system.pipeline_layout,
                    0,
                    vec![&self.draw_system.sprites_desc_set],
                    &[],
                );

                self.sprites.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

                for (_, sprite) in &self.sprites {
                    encoder.push_graphics_constants(
                        &self.draw_system.pipeline_layout,
                        ShaderStageFlags::VERTEX,
                        0,
                        utils::push_constant_data(sprite),
                    );

                    encoder.draw(0..6, 0..1);
                }

                self.glyphs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

                for (_, glyph) in &self.glyphs {
                    use rusttype::{Point};

                    let scale = glyph.scale();
                    let font_id = 0; // TODO: use the actual font id
                    let ascent = self.draw_system.fonts[font_id].v_metrics(scale).ascent;
                    let texcoords = self.draw_system.glyph_cache.rect_for(font_id, glyph).unwrap();

                    if let Some((uv_rect, px_rect)) = texcoords {
                        let glyph_sprite = {

                            let Point { x, y } = px_rect.min;
                            let w = px_rect.width() as f32;
                            let h = px_rect.height() as f32;

                            let Point { x: u, y: v } = uv_rect.min;
                            let uw = uv_rect.width();
                            let vh = uv_rect.height();

                            SpritePushConstants {
                                transform: make_transform(
                                    [x as f32, y as f32 + ascent],
                                    [w, h],
                                    [res_x as f32, res_y as f32]),
                                tint: [1.0, 1.0, 1.0, 1.0],
                                uv_origin: [u, v + 0.5],
                                uv_scale: [uw, vh / 2.0],
                            }
                        };

                        encoder.push_graphics_constants(
                            &self.draw_system.pipeline_layout,
                            ShaderStageFlags::VERTEX,
                            0,
                            utils::push_constant_data(&glyph_sprite),
                        );

                        encoder.draw(0..6, 0..1);
                    }
                }
            }

            command_buffer.finish()
        };

        let scene_submission = Submission::new()
            .wait_on(&[(
                &self.draw_system.frame_semaphore,
                PipelineStage::BOTTOM_OF_PIPE,
            )]).signal(&[&self.draw_system.scene_semaphore])
            .submit(vec![scene_command_buffer]);

        let blit_submission = Submission::new()
            .wait_on(&[(
                &self.draw_system.scene_semaphore,
                PipelineStage::BOTTOM_OF_PIPE,
            )]).signal(&[&self.draw_system.present_semaphore])
            .submit(vec![self.blit_command_buffer.take().unwrap()]);

        self.draw_system.queue_group.queues[0].submit(scene_submission, None);
        self.draw_system.queue_group.queues[0].submit(blit_submission, None);

        let result = swapchain.present(
            &mut self.draw_system.queue_group.queues[0],
            self.frame_index,
            vec![&self.draw_system.present_semaphore],
        );

        if result.is_err() {
            self.draw_system.swapchain_invalidated = true;
        }

        self.finished = true;
    }
}

impl<'a> Drop for JamBrushRenderer<'a> {
    fn drop(&mut self) {
        if !self.finished {
            panic!("JamBrushRenderer dropped without calling `finish()`");
        }
    }
}

fn make_transform(pos: [f32; 2], scale: [f32; 2], resolution: [f32; 2]) -> [[f32; 4]; 4] {
    let [sx, sy] = resolution;
    let [w, h] = scale;
    let [x, y] = pos;
    let dx = -1.0 + 2.0 * (x / sx as f32);
    let dy = -1.0 + 2.0 * (y / sy as f32);

    [
        [(w / sx as f32) * 2.0, 0.0, 0.0, 0.0],
        [0.0, (h / sy as f32) * 2.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [dx, dy, 0.0, 1.0],
    ]
}

