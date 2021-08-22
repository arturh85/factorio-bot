use std::path::Path;
use std::sync::Arc;

use actix_web::{web, HttpResponse};
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbaImage};

use crate::factorio::world::FactorioWorld;
use crate::types::ChunkPosition;

// use std::time::Instant;

const TILE_WIDTH: u32 = 256;
const TILE_HEIGHT: u32 = 256;

pub async fn map_tiles(
    world: web::Data<Arc<FactorioWorld>>,
    info: web::Path<(i32, i32, i32)>,
) -> Result<HttpResponse, actix_web::Error> {
    let (tile_z, tile_x, tile_y) = info.into_inner();
    let ((top_left_x, top_left_y), (bottom_right_x, _bottom_right_y)) =
        chunk_zoom(tile_z, tile_x, tile_y);
    let mut buffer: RgbaImage = image::ImageBuffer::new(TILE_WIDTH, TILE_HEIGHT);
    for (_x, _y, pixel) in buffer.enumerate_pixels_mut() {
        *pixel = image::Rgba([255, 255, 255, 255u8]);
    }
    let chunks_in_row: f64 = bottom_right_x - top_left_x;
    let chunk_width = TILE_WIDTH as f64 / chunks_in_row;
    // if a chunk is not even one pixel, no details shown
    if chunk_width > 1.0 {
        if chunks_in_row >= 8. {
            for chunk_ix in 0..((chunks_in_row / 8.).ceil() as u32) {
                for chunk_iy in 0..((chunks_in_row / 8.).ceil() as u32) {
                    let chunk_position = ChunkPosition {
                        x: top_left_x.floor() as i32 + (chunk_ix as i32 * 8),
                        y: top_left_y.floor() as i32 + (chunk_iy as i32 * 8),
                    };
                    let chunk_px = (chunk_ix as f64 * (chunk_width * 8.)).floor() as i32;
                    let chunk_py = (chunk_iy as f64 * (chunk_width * 8.)).floor() as i32;
                    let graphics_path_str = format!(
                        "workspace/client1/script-output/tiles/bigtile{}_{}.png",
                        chunk_position.x * 32,
                        chunk_position.y * 32
                    );
                    let img = match world.image_cache.get(&graphics_path_str) {
                        Some(img) => Some(img),
                        None => {
                            let graphics_path = Path::new(&graphics_path_str);
                            if graphics_path.exists() {
                                let img = image::open(graphics_path).unwrap().into_rgba8();
                                world
                                    .image_cache
                                    .insert(graphics_path_str.clone(), Box::new(img));
                                world.image_cache.get(&graphics_path_str)
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(img) = img {
                        let img = image::imageops::resize(
                            &**img,
                            (chunk_width * 8.) as u32,
                            (chunk_width * 8.) as u32,
                            FilterType::Nearest,
                        );
                        image::imageops::overlay(
                            &mut buffer,
                            &img,
                            chunk_px as u32,
                            chunk_py as u32,
                        );
                    }
                }
            }
        } else if chunks_in_row >= 1. {
            for chunk_ix in 0..(chunks_in_row.ceil() as u32) {
                for chunk_iy in 0..(chunks_in_row.ceil() as u32) {
                    let chunk_position = ChunkPosition {
                        x: top_left_x.floor() as i32 + chunk_ix as i32,
                        y: top_left_y.floor() as i32 + chunk_iy as i32,
                    };
                    let chunk_px = (chunk_ix as f64 * chunk_width).floor() as i32;
                    let chunk_py = (chunk_iy as f64 * chunk_width).floor() as i32;

                    let graphics_path_str = format!(
                        "workspace/client1/script-output/tiles/tile{}_{}.png",
                        chunk_position.x * 32,
                        chunk_position.y * 32
                    );
                    let img = match world.image_cache.get(&graphics_path_str) {
                        Some(img) => Some(img),
                        None => {
                            let graphics_path = Path::new(&graphics_path_str);
                            if graphics_path.exists() {
                                let img = image::open(graphics_path).unwrap().into_rgba8();
                                world
                                    .image_cache
                                    .insert(graphics_path_str.clone(), Box::new(img));
                                world.image_cache.get(&graphics_path_str)
                            } else {
                                None
                            }
                        }
                    };

                    if let Some(img) = img {
                        let img = image::imageops::resize(
                            &**img,
                            chunk_width as u32,
                            chunk_width as u32,
                            FilterType::Nearest,
                        );
                        image::imageops::overlay(
                            &mut buffer,
                            &img,
                            chunk_px as u32,
                            chunk_py as u32,
                        );
                    }
                }
            }
        } else {
            let top_left_x = top_left_x * 32.;
            let top_left_y = top_left_y * 32.;

            let chunk_x = if top_left_x > 0. {
                top_left_x as i32 - (top_left_x.abs() as u32 % 32) as i32
            } else {
                top_left_x as i32 - ((32 - (top_left_x.abs() as i32 % 32)) % 32)
            };
            let chunk_y = if top_left_y > 0. {
                top_left_y as i32 - (top_left_y.abs() as u32 % 32) as i32
            } else {
                top_left_y as i32 - ((32 - (top_left_y.abs() as i32 % 32)) % 32)
            };

            let graphics_path_str = format!(
                "workspace/client1/script-output/tiles/tile{}_{}.png",
                chunk_x, chunk_y
            );

            let img = match world.image_cache.get(&graphics_path_str) {
                Some(img) => Some(img),
                None => {
                    // if let Some(img) = writer.get_one(&graphics_path_str) {
                    //     drop(writer);
                    //     world.image_cache.get_one(&graphics_path_str)
                    // } else {
                    let graphics_path = Path::new(&graphics_path_str);
                    if graphics_path.exists() {
                        let img = image::open(graphics_path).unwrap().into_rgba8();
                        world
                            .image_cache
                            .insert(graphics_path_str.clone(), Box::new(img));
                        world.image_cache.get(&graphics_path_str)
                    } else {
                        None
                    }
                    // }
                }
            };

            let ix = ((top_left_x - chunk_x as f64) / (chunks_in_row * 32.)) as i32;
            let iy = ((top_left_y - chunk_y as f64) / (chunks_in_row * 32.)) as i32;

            if let Some(img) = img {
                let img = image::imageops::crop_imm(
                    &**img,
                    ((512. * chunks_in_row) * ix as f64) as u32,
                    ((512. * chunks_in_row) * iy as f64) as u32,
                    (512. * chunks_in_row) as u32,
                    (512. * chunks_in_row) as u32,
                );
                let img =
                    image::imageops::resize(&img, TILE_WIDTH, TILE_HEIGHT, FilterType::Nearest);
                image::imageops::overlay(&mut buffer, &img, 0, 0);
            } else {
                warn!("tile not found: {}", graphics_path_str);
            }
        }
    }
    let dynamic = DynamicImage::ImageRgba8(buffer);
    let mut buf: Vec<u8> = Vec::new();
    dynamic
        .write_to(&mut buf, ImageFormat::Png)
        .expect("failed to write image");
    // info!("image writing took <yellow>{:?}</>", started.elapsed());
    // Content(ContentType::PNG, buf)

    Ok(HttpResponse::Ok().content_type("image/png").body(buf))
}

pub fn chunk_zoom(z: i32, x: i32, y: i32) -> ((f64, f64), (f64, f64)) {
    // one chunk is 32x32 positions big
    let map_size_chunks = 32f64; // map must be a certain size
    let map_size_chunks_half = map_size_chunks / 2.0; // map must be a certain size

    // from -16 to +16

    let x = x as f64;
    let y = y as f64;

    // z = 0, zoom_width = 32
    // z = 1, zoom_width = 16

    // -8 = -16 + (1 * 8)
    // +8 = -16 + (0 * 8)

    let zoom_width = map_size_chunks / 2.0f64.powi(z);
    let top_left = (
        (-map_size_chunks_half + (zoom_width * x)) as f64,
        (-map_size_chunks_half + (zoom_width * y)) as f64,
    );
    let bottom_right = (
        (-map_size_chunks_half + (zoom_width * (x + 1.0f64))) as f64,
        (-map_size_chunks_half + (zoom_width * (y + 1.0f64))) as f64,
    );

    (top_left, bottom_right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_zoom_0() {
        let (zoom_world_top_left, zoom_world_bottom_right) = chunk_zoom(0, 0, 0);
        assert_eq!(zoom_world_top_left, (-16.0, -16.0));
        assert_eq!(zoom_world_bottom_right, (16.0, 16.0));
    }

    #[test]
    fn test_chunk_zoom_1() {
        let (zoom_world_top_left, zoom_world_bottom_right) = chunk_zoom(1, 0, 0);
        assert_eq!(zoom_world_top_left, (-16.0, -16.0));
        assert_eq!(zoom_world_bottom_right, (0.0, 0.0));
    }
}
