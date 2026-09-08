//! Pre-rasterized Lucide icons: decode only the small masks actually displayed.
use couch_model::Icon;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::{cell::RefCell, collections::HashMap};
const ALPHA: &[u8] = include_bytes!("../assets/lucide.alpha");
const PIXELS: usize = 24 * 24;
const _: () = assert!(ALPHA.len() == couch_model::ALL_ICONS.len() * PIXELS);
thread_local! { static CACHE: RefCell<HashMap<Icon, Image>> = RefCell::new(HashMap::new()); }
pub fn image(icon: Icon) -> Image {
    CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(icon)
            .or_insert_with(|| {
                let start = icon.catalog_index() * PIXELS;
                let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(24, 24);
                for (pixel, alpha) in pixels
                    .make_mut_bytes()
                    .chunks_exact_mut(4)
                    .zip(&ALPHA[start..start + PIXELS])
                {
                    pixel.copy_from_slice(&[255, 255, 255, *alpha]);
                }
                Image::from_rgba8(pixels)
            })
            .clone()
    })
}
