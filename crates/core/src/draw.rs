use imageproc::drawing::{Canvas, draw_hollow_rect_mut, draw_line_segment_mut};

use crate::factorio::util::{
    scaled_draw_rect, vector_add, vector_multiply, vector_normalize, vector_substract,
};
use crate::factorio::world::FactorioWorld;
use crate::graph::entity_graph::{BlockedQuadTree, ResourceQuadTree};
use crate::types::{Position, Rect};
use image::RgbaImage;
use miette::{IntoDiagnostic, Result};
use parking_lot::RwLockReadGuard;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

//Hallo Welt!

#[allow(clippy::clone_on_copy, clippy::cast_lossless)]
pub fn arrow_mut<C>(canvas: &mut C, start: (f32, f32), end: (f32, f32), color: C::Pixel, size: f64)
where
    C: Canvas,
    C::Pixel: 'static,
{
    draw_line_segment_mut(canvas, start, end, color.clone());
    // from: https://stackoverflow.com/questions/10316180/how-to-calculate-the-coordinates-of-a-arrowhead-based-on-the-arrow
    if size > 1. {
        let h = size * 3.0_f64.sqrt();
        let w = size.clone();
        let start_position = Position::new(start.0.clone() as f64, start.1.clone() as f64);
        let end_position = Position::new(end.0.clone() as f64, end.1.clone() as f64);
        let u = vector_normalize(&vector_substract(&end_position, &start_position));
        let vw = vector_multiply(&Position::new(-u.y(), u.x()), w);
        let vv = vector_substract(&end_position, &vector_multiply(&u, h));
        let v1 = vector_add(&vv, &vw);
        let v2 = vector_substract(&vv, &vw);
        draw_line_segment_mut(
            canvas,
            end.clone(),
            (v1.x() as f32, v1.y() as f32),
            color.clone(),
        );
        draw_line_segment_mut(
            canvas,
            end.clone(),
            (v2.x() as f32, v2.y() as f32),
            color.clone(),
        );
        draw_line_segment_mut(
            canvas,
            (v1.x() as f32, v1.y() as f32),
            (v2.x() as f32, v2.y() as f32),
            color.clone(),
        );
    }
}

#[allow(clippy::clone_on_copy, clippy::cast_lossless)]
pub fn draw_blocked_rects_mut<C>(
    canvas: &mut C,
    blocked: RwLockReadGuard<BlockedQuadTree>,
    bounding_box: &Rect,
    scaling_factor: f64,
    color_mineable: C::Pixel,
    color_unmineable: C::Pixel,
) where
    C: Canvas,
    C::Pixel: 'static,
{
    for (minable, rect, _id) in blocked.query(bounding_box.clone().into()) {
        if let Some(draw_rect) = scaled_draw_rect(bounding_box, rect, scaling_factor.clone()) {
            draw_hollow_rect_mut(
                canvas,
                draw_rect,
                if *minable {
                    color_mineable.clone()
                } else {
                    color_unmineable.clone()
                },
            );
        }
    }
}

#[allow(clippy::clone_on_copy)]
pub fn draw_resource_rects_mut<C>(
    canvas: &mut C,
    resources: RwLockReadGuard<ResourceQuadTree>,
    bounding_box: &Rect,
    scaling_factor: f64,
    colors: HashMap<&str, C::Pixel>,
    invalid_color: C::Pixel,
) where
    C: Canvas,
    C::Pixel: 'static,
{
    for (name, rect, _id) in resources.query(bounding_box.clone().into()) {
        if let Some(draw_rect) = scaled_draw_rect(bounding_box, rect, scaling_factor.clone()) {
            draw_hollow_rect_mut(
                canvas,
                draw_rect,
                colors.get(name.as_str()).unwrap_or(&invalid_color).clone(),
            );
        }
    }
}

/// Renders the world's resource and blocked quad-trees to a PNG at `save_path`.
///
/// `save_path` must already be resolved and bounded by the caller: this is
/// reachable from the `world.draw` Lua binding, so the path in it is
/// attacker-controlled. See `factorio_bot_core::scripts::resolve_write_path`.
pub fn draw_world(world: Arc<FactorioWorld>, save_path: &Path) -> Result<()> {
    let image_width = 500.;
    let image_height = 500.;
    let bb_width = 200.;
    let bb_height = 200.;
    let mut buffer: RgbaImage = image::ImageBuffer::new(image_width as u32, image_height as u32);
    for (_x, _y, pixel) in buffer.enumerate_pixels_mut() {
        *pixel = image::Rgba([255, 255, 255, 255u8]); // white
    }
    let bounding_box = Rect::from_wh(bb_width, bb_height);
    let scaling_factor = image_width / bb_width;
    let resource_colors: HashMap<&str, image::Rgba<_>> = [
        ("iron-ore", image::Rgba([0u8, 110u8, 255u8, 255u8])), // blue
        ("copper-ore", image::Rgba([255u8, 55u8, 0u8, 255u8])), // orange
        ("coal", image::Rgba([0u8, 0u8, 0u8, 255u8])),         // black
        ("stone", image::Rgba([150u8, 100u8, 80u8, 255u8])),   // brown
        ("uranium-ore", image::Rgba([100u8, 180u8, 0u8, 255u8])), // fancy green
        ("crude-oil", image::Rgba([255u8, 0u8, 255u8, 255u8])), // magenta
    ]
    .iter()
    .cloned()
    .collect();
    // draw resources
    draw_resource_rects_mut(
        &mut buffer,
        world.entity_graph.resource_tree(),
        &bounding_box,
        scaling_factor,
        resource_colors,
        image::Rgba([255u8, 0u8, 255u8, 255u8]), // magenta
    );
    // draw blocked
    draw_blocked_rects_mut(
        &mut buffer,
        world.entity_graph.blocked_tree(),
        &bounding_box,
        scaling_factor,
        image::Rgba([76u8, 175u8, 80u8, 255u8]),  // green
        image::Rgba([93u8, 247u8, 255u8, 255u8]), // cyan
    );
    // draw thick vertical black line through the center
    draw_hollow_rect_mut(
        &mut buffer,
        imageproc::rect::Rect::at((image_width / 2. - 1.) as i32, 0)
            .of_size(2, image_height as u32),
        image::Rgba([0u8, 0u8, 0u8, 255u8]), // black
    );
    // draw thick horizontal black line through the center
    draw_hollow_rect_mut(
        &mut buffer,
        imageproc::rect::Rect::at(0, (image_height / 2. - 1.) as i32)
            .of_size(image_width as u32, 2),
        image::Rgba([0u8, 0u8, 0u8, 255u8]), // black
    );
    buffer.save(save_path).into_diagnostic()
}
