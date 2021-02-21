use imageproc::drawing::{draw_hollow_rect_mut, draw_line_segment_mut, Canvas};

use crate::factorio::entity_graph::{BlockedQuadTree, ResourceQuadTree};
use crate::factorio::util::{
    scaled_draw_rect, vector_add, vector_multiply, vector_normalize, vector_substract,
};
use crate::types::{Position, Rect};
use dashmap::lock::RwLockReadGuard;
use std::collections::HashMap;

#[allow(clippy::clone_on_copy)]
pub fn draw_arrow_mut<C>(
    canvas: &mut C,
    start: (f32, f32),
    end: (f32, f32),
    color: C::Pixel,
    size: f64,
) where
    C: Canvas,
    C::Pixel: 'static,
{
    draw_line_segment_mut(canvas, start, end, color.clone());
    // from: https://stackoverflow.com/questions/10316180/how-to-calculate-the-coordinates-of-a-arrowhead-based-on-the-arrow
    if size > 1. {
        let h = size * 3.0f64.sqrt();
        let w = size;
        let start_position = Position::new(start.0 as f64, start.1 as f64);
        let end_position = Position::new(end.0 as f64, end.1 as f64);
        let u = vector_normalize(&vector_substract(&end_position, &start_position));
        let vw = vector_multiply(&Position::new(-u.y(), u.x()), w);
        let vv = vector_substract(&end_position, &vector_multiply(&u, h));
        let v1 = vector_add(&vv, &vw);
        let v2 = vector_substract(&vv, &vw);
        draw_line_segment_mut(canvas, end, (v1.x() as f32, v1.y() as f32), color.clone());
        draw_line_segment_mut(canvas, end, (v2.x() as f32, v2.y() as f32), color.clone());
        draw_line_segment_mut(
            canvas,
            (v1.x() as f32, v1.y() as f32),
            (v2.x() as f32, v2.y() as f32),
            color.clone(),
        );
    }
}

#[allow(clippy::clone_on_copy)]
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
        if let Some(draw_rect) = scaled_draw_rect(bounding_box, rect, scaling_factor) {
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
        if let Some(draw_rect) = scaled_draw_rect(bounding_box, rect, scaling_factor) {
            draw_hollow_rect_mut(
                canvas,
                draw_rect,
                colors.get(name.as_str()).unwrap_or(&invalid_color).clone(),
            );
        }
    }
}
