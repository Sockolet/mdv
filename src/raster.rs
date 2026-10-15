use eframe::egui;
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use raster_egui as raster;

pub struct Painter(EguiSoftwareRender);

impl Default for Painter {
    fn default() -> Self {
        Self(EguiSoftwareRender::new(ColorFieldOrder::Bgra))
    }
}

impl Painter {
    pub fn paint(
        &mut self,
        buffer: &mut [u32],
        size: [usize; 2],
        primitives: &[egui::ClippedPrimitive],
        textures: &egui::TexturesDelta,
        pixels_per_point: f32,
    ) -> Result<(), String> {
        let primitives = primitives
            .iter()
            .map(|primitive| {
                let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
                    return Err(
                        "GPU paint callbacks cannot be used by the CPU renderer.".to_owned()
                    );
                };
                Ok(raster::ClippedPrimitive {
                    clip_rect: raster::Rect::from_min_max(
                        raster::pos2(primitive.clip_rect.min.x, primitive.clip_rect.min.y),
                        raster::pos2(primitive.clip_rect.max.x, primitive.clip_rect.max.y),
                    ),
                    primitive: raster::epaint::Primitive::Mesh(raster::Mesh {
                        indices: mesh.indices.clone(),
                        vertices: mesh
                            .vertices
                            .iter()
                            .map(|vertex| raster::epaint::Vertex {
                                pos: raster::pos2(vertex.pos.x, vertex.pos.y),
                                uv: raster::pos2(vertex.uv.x, vertex.uv.y),
                                color: color(vertex.color),
                            })
                            .collect(),
                        texture_id: texture_id(mesh.texture_id),
                    }),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        // The rasterizer's stable release uses egui 0.34. Only paint data crosses
        // this boundary; the application and input handling stay on egui 0.36.
        let textures = raster::TexturesDelta {
            set: textures
                .set
                .iter()
                .flat_map(|(id, deltas)| {
                    deltas.iter().map(move |delta| {
                        let egui::ImageData::Color(image) = &delta.image;
                        (
                            texture_id(*id),
                            raster::epaint::ImageDelta {
                                image: raster::ImageData::Color(
                                    raster::ColorImage::new(
                                        image.size,
                                        image.pixels.iter().copied().map(color).collect(),
                                    )
                                    .into(),
                                ),
                                options: raster::TextureOptions {
                                    magnification: filter(delta.options.magnification),
                                    minification: filter(delta.options.minification),
                                    wrap_mode: match delta.options.wrap_mode {
                                        egui::TextureWrapMode::ClampToEdge => {
                                            raster::TextureWrapMode::ClampToEdge
                                        }
                                        egui::TextureWrapMode::Repeat => {
                                            raster::TextureWrapMode::Repeat
                                        }
                                        egui::TextureWrapMode::MirroredRepeat => {
                                            raster::TextureWrapMode::MirroredRepeat
                                        }
                                    },
                                    mipmap_mode: delta.options.mipmap_mode.map(filter),
                                },
                                pos: delta.pos,
                            },
                        )
                    })
                })
                .collect(),
            free: textures.free.iter().copied().map(texture_id).collect(),
        };
        buffer.fill(0);
        self.0.render(
            &mut BufferMutRef::new(bytemuck::cast_slice_mut(buffer), size[0], size[1]),
            &primitives,
            &textures,
            pixels_per_point,
        );
        Ok(())
    }
}

fn color(color: egui::Color32) -> raster::Color32 {
    raster::Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), color.a())
}

fn texture_id(id: egui::TextureId) -> raster::TextureId {
    match id {
        egui::TextureId::Managed(id) => raster::TextureId::Managed(id),
        egui::TextureId::User(id) => raster::TextureId::User(id),
    }
}

fn filter(filter: egui::TextureFilter) -> raster::TextureFilter {
    match filter {
        egui::TextureFilter::Linear => raster::TextureFilter::Linear,
        egui::TextureFilter::Nearest => raster::TextureFilter::Nearest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(id: egui::TextureId, clip: egui::Rect) -> egui::ClippedPrimitive {
        let mut mesh = egui::Mesh::with_texture(id);
        mesh.add_rect_with_uv(
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(8.0, 8.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        egui::ClippedPrimitive {
            clip_rect: clip,
            primitive: egui::epaint::Primitive::Mesh(mesh),
        }
    }

    #[test]
    fn raster_bridge_preserves_color_channels_and_clip_rectangles() {
        let id = egui::TextureId::Managed(7);
        let mut textures = egui::TexturesDelta::default();
        textures
            .set
            .entry(id)
            .or_default()
            .push(egui::epaint::ImageDelta::full(
                egui::ColorImage::new([1, 1], vec![egui::Color32::RED]),
                egui::TextureOptions::NEAREST,
            ));
        let clip = egui::Rect::from_min_max(egui::pos2(2.0, 2.0), egui::pos2(6.0, 6.0));
        let mut pixels = vec![0; 64];
        Painter::default()
            .paint(&mut pixels, [8, 8], &[rectangle(id, clip)], &textures, 1.0)
            .unwrap();
        textures.clear();
        assert_eq!(pixels[3 * 8 + 3] & 0x00ff_ffff, 0x00ff_0000);
        assert_eq!(pixels[0], 0);
        assert_eq!(pixels[7 * 8 + 7], 0);
    }

    #[test]
    fn raster_bridge_applies_every_texture_patch_in_order() {
        let id = egui::TextureId::Managed(8);
        let mut textures = egui::TexturesDelta::default();
        let patches = textures.set.entry(id).or_default();
        patches.push(egui::epaint::ImageDelta::full(
            egui::ColorImage::new([2, 1], vec![egui::Color32::RED, egui::Color32::BLUE]),
            egui::TextureOptions::NEAREST,
        ));
        patches.push(egui::epaint::ImageDelta::partial(
            [1, 0],
            egui::ColorImage::new([1, 1], vec![egui::Color32::GREEN]),
            egui::TextureOptions::NEAREST,
        ));
        patches.push(egui::epaint::ImageDelta::partial(
            [0, 0],
            egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
            egui::TextureOptions::NEAREST,
        ));
        let mut pixels = vec![0; 64];
        Painter::default()
            .paint(
                &mut pixels,
                [8, 8],
                &[rectangle(id, egui::Rect::EVERYTHING)],
                &textures,
                1.0,
            )
            .unwrap();
        textures.clear();
        assert_eq!(pixels[2 * 8 + 1] & 0x00ff_ffff, 0x00ff_ffff);
        assert_eq!(pixels[2 * 8 + 6] & 0x00ff_ffff, 0x0000_ff00);
    }
}
