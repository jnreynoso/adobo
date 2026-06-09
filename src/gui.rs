use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};
use softbuffer::{Context, Surface};
use tiny_skia::*;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::Arc;
use ab_glyph::{FontVec, Font};
use crate::interpreter::DrawCommand;
use crate::object::PdfObject;


#[derive(Debug, Clone)]
pub struct PageInfo {
    pub width: f32,
    pub height: f32,
    pub top_y: f32,
}

pub struct RenderRequest {
    pub page_idx: usize,
    pub zoom: f32,
    pub page_y: f32,
    pub page_height: f32,
    pub window_height: f32,
}

pub enum WorkerMessage {
    PageRendered {
        page_idx: usize,
        zoom: f32,
        pixmap: Pixmap,
    },
}

struct App {
    pages: Vec<PageInfo>,
    window: Option<Rc<Window>>,
    context: Option<Context<Rc<Window>>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    page_images: RefCell<std::collections::HashMap<usize, Pixmap>>,
    requested_pages: RefCell<std::collections::HashSet<(usize, u32)>>,
    page_access_order: RefCell<std::collections::VecDeque<usize>>,
    tx_worker: std::sync::mpsc::Sender<RenderRequest>,
    rx_worker: std::sync::mpsc::Receiver<WorkerMessage>,
    scroll_x: f32,
    scroll_y: f32,
    target_scroll_x: f32,
    target_scroll_y: f32,
    last_scroll_y: f32,
    scroll_down_direction: bool,
    zoom: f32,
    modifiers: winit::keyboard::ModifiersState,
    mouse_pos: (f32, f32),
    window_size: winit::dpi::PhysicalSize<u32>,
    zoom_initialized: bool,
    default_font: Arc<FontVec>,
    logo_pixmap: Option<Pixmap>,
}

fn load_window_icon() -> Option<winit::window::Icon> {
    if let Ok(pixmap) = Pixmap::load_png("logo.png") {
        let target_size = 64;
        let mut resized = Pixmap::new(target_size, target_size)?;
        let sx = target_size as f32 / pixmap.width() as f32;
        let sy = target_size as f32 / pixmap.height() as f32;
        let transform = Transform::from_scale(sx, sy);
        resized.draw_pixmap(
            0, 0,
            pixmap.as_ref(),
            &PixmapPaint::default(),
            transform,
            None
        );
        let mut rgba = resized.data().to_vec();
        for pixel in rgba.chunks_exact_mut(4) {
            let a = pixel[3];
            if a > 0 && a < 255 {
                pixel[0] = ((pixel[0] as u16 * 255) / a as u16) as u8;
                pixel[1] = ((pixel[1] as u16 * 255) / a as u16) as u8;
                pixel[2] = ((pixel[2] as u16 * 255) / a as u16) as u8;
            }
        }
        winit::window::Icon::from_rgba(rgba, target_size, target_size).ok()
    } else {
        None
    }
}

impl App {
    fn center_on_content(&mut self, width: u32, height: u32) {
        if !self.pages.is_empty() {
            let page_w = self.pages[0].width;
            let page_h = self.pages[0].height;
            
            let cx = (width as f32 / 2.0) - ((page_w / 2.0) * self.zoom);
            let cy;
            let page_h_zoomed = page_h * self.zoom;
            if page_h_zoomed < height as f32 {
                cy = (height as f32 - page_h_zoomed) / 2.0;
            } else {
                cy = 20.0;
            }
            
            self.scroll_x = cx;
            self.scroll_y = cy;
            self.target_scroll_x = cx;
            self.target_scroll_y = cy;
            self.last_scroll_y = cy;
        } else {
            self.scroll_x = 100.0;
            self.scroll_y = 100.0;
            self.target_scroll_x = 100.0;
            self.target_scroll_y = 100.0;
            self.last_scroll_y = 100.0;
        }
    }

    fn calculate_fit_zoom(&self, width: u32, height: u32) -> f32 {
        if self.pages.is_empty() || width == 0 || height == 0 {
            return 1.0;
        }
        let page = &self.pages[0];
        let margin = 40.0; // 20px on each side
        let zoom_w = (width as f32 - margin) / page.width;
        let zoom_h = (height as f32 - margin) / page.height;
        zoom_w.min(zoom_h).clamp(0.1, 10.0)
    }

    fn draw_text(&self, pixmap: &mut Pixmap, text: &str, start_x: f32, y: f32, size: f32, font: &FontVec, paint: &Paint) {
        let scale_factor = size / font.units_per_em().unwrap_or(1000.0);
        let mut current_x = start_x;
        for c in text.chars() {
            let glyph_id = font.glyph_id(c);
            let actual_w = font.h_advance_unscaled(glyph_id) * scale_factor;
            
            if let Some(outline) = font.outline(glyph_id) {
                let mut path_builder = PathBuilder::new();
                let mut last_point: Option<ab_glyph::Point> = None;
                for curve in outline.curves {
                    let start_p = match curve {
                        ab_glyph::OutlineCurve::Line(p1, _) => p1,
                        ab_glyph::OutlineCurve::Quad(p1, _, _) => p1,
                        ab_glyph::OutlineCurve::Cubic(p1, _, _, _) => p1,
                    };
                    let is_new_contour = match last_point {
                        Some(lp) => (start_p.x - lp.x).abs() > 0.001 || (start_p.y - lp.y).abs() > 0.001,
                        None => true,
                    };
                    if is_new_contour {
                        path_builder.move_to(current_x + start_p.x * scale_factor, y - start_p.y * scale_factor);
                    }
                    match curve {
                        ab_glyph::OutlineCurve::Line(_, p2) => {
                            path_builder.line_to(current_x + p2.x * scale_factor, y - p2.y * scale_factor);
                            last_point = Some(p2);
                        }
                        ab_glyph::OutlineCurve::Quad(_, p2, p3) => {
                            path_builder.quad_to(current_x + p2.x * scale_factor, y - p2.y * scale_factor,
                                               current_x + p3.x * scale_factor, y - p3.y * scale_factor);
                            last_point = Some(p3);
                        }
                        ab_glyph::OutlineCurve::Cubic(_, p2, p3, p4) => {
                            path_builder.cubic_to(current_x + p2.x * scale_factor, y - p2.y * scale_factor,
                                                current_x + p3.x * scale_factor, y - p3.y * scale_factor,
                                                current_x + p4.x * scale_factor, y - p4.y * scale_factor);
                            last_point = Some(p4);
                        }
                    }
                }
                if let Some(path) = path_builder.finish() {
                    pixmap.fill_path(&path, paint, FillRule::Winding, Transform::identity(), None);
                }
            }
            current_x += actual_w;
        }
    }

    fn measure_text_width(&self, text: &str, size: f32, font: &FontVec) -> f32 {
        let scale_factor = size / font.units_per_em().unwrap_or(1000.0);
        let mut width = 0.0;
        for c in text.chars() {
            let glyph_id = font.glyph_id(c);
            width += font.h_advance_unscaled(glyph_id) * scale_factor;
        }
        width
    }

    fn record_access_only(&self, page_idx: usize) {
        let mut order = self.page_access_order.borrow_mut();
        if let Some(pos) = order.iter().position(|&x| x == page_idx) {
            order.remove(pos);
        }
        order.push_back(page_idx);
    }

    fn record_access_and_evict(&self, page_idx: usize) {
        {
            let mut order = self.page_access_order.borrow_mut();
            if let Some(pos) = order.iter().position(|&x| x == page_idx) {
                order.remove(pos);
            }
            order.push_back(page_idx);
        }

        let mut images = self.page_images.borrow_mut();
        let mut order = self.page_access_order.borrow_mut();
        while images.len() > 12 {
            if let Some(lru_idx) = order.pop_front() {
                images.remove(&lru_idx);
            } else {
                break;
            }
        }
    }

    fn clear_cache(&self) {
        self.page_images.borrow_mut().clear();
        self.requested_pages.borrow_mut().clear();
        self.page_access_order.borrow_mut().clear();
    }

    fn draw_splash_screen(&self, buffer: &mut [u32], width: usize, height: usize) {
        let bg_color = (18u32 << 16) | (18u32 << 8) | 18u32;
        buffer.fill(bg_color);

        let font = &self.default_font;
        let title = "UfReader";
        let title_size = 36.0f32;
        let tp = {
            let mut p = Paint::default();
            p.set_color_rgba8(255, 255, 255, 255);
            p.anti_alias = true;
            p
        };

        let splash_w = 400;
        let splash_h = 400;
        if let Some(mut sp) = Pixmap::new(splash_w, splash_h) {
            sp.fill(Color::from_rgba8(18, 18, 18, 255));

            if let Some(ref logo_pixmap) = self.logo_pixmap {
                let lw = 160.0f32;
                let lh = 160.0f32;
                let lx = (splash_w as f32 - lw) / 2.0;
                let ly = 40.0f32;
                let sx = lw / logo_pixmap.width() as f32;
                let sy = lh / logo_pixmap.height() as f32;
                let transform = Transform::from_scale(sx, sy);
                sp.draw_pixmap(
                    lx as i32, ly as i32,
                    logo_pixmap.as_ref(),
                    &PixmapPaint::default(),
                    transform,
                    None
                );
            }

            let text_y = 260.0f32;
            let tw = self.measure_text_width(title, title_size, font);
            let tx = (splash_w as f32 - tw) / 2.0;
            self.draw_text(&mut sp, title, tx, text_y, title_size, font, &tp);

            let sub = "Cargando documento...";
            let sub_size = 16.0f32;
            let sp_sub = {
                let mut p = Paint::default();
                p.set_color_rgba8(150, 150, 150, 255);
                p.anti_alias = true;
                p
            };
            let sw = self.measure_text_width(sub, sub_size, font);
            let sx = (splash_w as f32 - sw) / 2.0;
            self.draw_text(&mut sp, sub, sx, text_y + 40.0, sub_size, font, &sp_sub);

            let sp_data = sp.data();
            let sp_u32: &[u32] = unsafe {
                std::slice::from_raw_parts(sp_data.as_ptr() as *const u32, sp_data.len() / 4)
            };
            let ox = (width.saturating_sub(splash_w as usize)) / 2;
            let oy = (height.saturating_sub(splash_h as usize)) / 2;

            for row in 0..splash_h as usize {
                let dst_row = oy + row;
                if dst_row >= height { break; }
                let base = dst_row * width;
                for col in 0..splash_w as usize {
                    let dst_col = ox + col;
                    if dst_col >= width { break; }
                    let s = sp_u32[row * splash_w as usize + col];
                    buffer[base + dst_col] = ((s & 0xFF) << 16) | (s & 0xFF00) | ((s >> 16) & 0xFF);
                }
            }
        }
    }

    fn get_hover_state(&self, mx: f32, my: f32) -> u8 {
        let width = self.window_size.width as f32;
        let height = self.window_size.height as f32;

        let overlay_width = 360.0;
        let overlay_height = 72.0;
        let overlay_x = width - overlay_width - 30.0;
        let overlay_y = height - overlay_height - 30.0;

        let is_overlay_focused = mx >= overlay_x - 30.0 
            && mx <= width + 10.0 
            && my >= overlay_y - 30.0 
            && my <= height + 10.0;

        if !is_overlay_focused {
            return 0;
        }

        let btn_y = overlay_y + 9.0;
        let btn_size = 54.0;

        // Minus button
        let minus_x = overlay_x + 15.0;
        if mx >= minus_x && mx <= minus_x + btn_size && my >= btn_y && my <= btn_y + btn_size {
            return 2;
        }

        // Plus button
        let plus_x = overlay_x + 222.0;
        if mx >= plus_x && mx <= plus_x + btn_size && my >= btn_y && my <= btn_y + btn_size {
            return 3;
        }

        // Reset button
        let reset_x = overlay_x + 291.0;
        if mx >= reset_x && mx <= reset_x + btn_size && my >= btn_y && my <= btn_y + btn_size {
            return 4;
        }

        1 // Overlay focused, but no button hovered
    }

    fn draw(&mut self, _window: &Window) {
        let width = self.window_size.width as usize;
        let height = self.window_size.height as usize;

        if width == 0 || height == 0 { return; }

        // Detect scroll direction
        let scroll_diff = self.target_scroll_y - self.last_scroll_y;
        if scroll_diff < -0.1 {
            self.scroll_down_direction = true;
        } else if scroll_diff > 0.1 {
            self.scroll_down_direction = false;
        }
        self.last_scroll_y = self.target_scroll_y;

        // No easing - scroll is applied immediately for zero lag
        self.scroll_x = self.target_scroll_x;
        self.scroll_y = self.target_scroll_y;

        // Resize surface (while still borrowing self)
        {
            let surface = match self.surface.as_mut() { Some(s) => s, None => return };
            let w = match NonZeroU32::new(width as u32) { Some(w) => w, None => return };
            let h = match NonZeroU32::new(height as u32) { Some(h) => h, None => return };
            if let Err(e) = surface.resize(w, h) { eprintln!("resize: {}", e); return; }
        }
        // Take surface out of self so we can borrow other fields (pages, page_images, etc.)
        // while also holding `buffer` (which borrows from surface, not self).
        let mut surface = match self.surface.take() { Some(s) => s, None => return };
        let mut buffer = match surface.buffer_mut() { Ok(b) => b, Err(e) => { eprintln!("buffer: {}", e); self.surface = Some(surface); return; } };

        // Background pixel in softbuffer XRGB format
        // tiny-skia: bytes [R,G,B,A] -> u32 little-endian = 0xAABBGGRR
        // softbuffer: expects 0x00RRGGBB
        // bg = R=82, G=86, B=89
        let bg: u32 = (82u32 << 16) | (86u32 << 8) | 89u32;
        let white: u32 = 0x00FFFFFF;

        // Evict pages far from viewport using LRU cache capacity
        // (Handled automatically on page insertion via record_access_and_evict)

        let page_count = self.pages.len();

        let is_loading = self.page_images.borrow().is_empty();
        if page_count > 0 && is_loading {
            // Find visible page range
            let first_visible = {
                let mut lo = 0usize;
                let mut hi = page_count;
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let bot = self.scroll_y + (self.pages[mid].top_y + self.pages[mid].height) * self.zoom;
                    if bot < -100.0 { lo = mid + 1; } else { hi = mid; }
                }
                lo
            };

            let mut next_non_visible = page_count;
            for page_idx in first_visible..page_count {
                let page = &self.pages[page_idx];
                let page_y = (self.scroll_y + page.top_y * self.zoom).round() as i32;
                if page_y >= height as i32 {
                    next_non_visible = page_idx;
                    break;
                }
            }

            // Send render requests for the visible pages so loading progresses
            let zoom_key = (self.zoom * 1000.0) as u32;
            {
                let mut requested = self.requested_pages.borrow_mut();
                for idx in first_visible..next_non_visible {
                    if !requested.contains(&(idx, zoom_key)) {
                        let page = &self.pages[idx];
                        let page_h_f = page.height * self.zoom;
                        let page_y = (self.scroll_y + page.top_y * self.zoom).round() as i32;
                        let page_h = page_h_f.round() as i32;
                        self.tx_worker.send(RenderRequest {
                            page_idx: idx,
                            zoom: self.zoom,
                            page_y: page_y as f32,
                            page_height: page_h as f32,
                            window_height: height as f32,
                        }).ok();
                        requested.insert((idx, zoom_key));
                    }
                }
            }

            // Send preload requests as well
            {
                let (preload_above, preload_below) = if self.scroll_down_direction {
                    (1, 3)
                } else {
                    (3, 1)
                };
                let mut requested = self.requested_pages.borrow_mut();

                let start_above = first_visible.saturating_sub(preload_above);
                for idx in start_above..first_visible {
                    if idx < page_count && !requested.contains(&(idx, zoom_key)) {
                        let page = &self.pages[idx];
                        let page_h_f = page.height * self.zoom;
                        let page_y = (self.scroll_y + page.top_y * self.zoom).round() as i32;
                        let page_h = page_h_f.round() as i32;
                        self.tx_worker.send(RenderRequest {
                            page_idx: idx,
                            zoom: self.zoom,
                            page_y: page_y as f32,
                            page_height: page_h as f32,
                            window_height: height as f32,
                        }).ok();
                        requested.insert((idx, zoom_key));
                    }
                }

                let end_below = (next_non_visible + preload_below).min(page_count);
                for idx in next_non_visible..end_below {
                    if idx < page_count && !requested.contains(&(idx, zoom_key)) {
                        let page = &self.pages[idx];
                        let page_h_f = page.height * self.zoom;
                        let page_y = (self.scroll_y + page.top_y * self.zoom).round() as i32;
                        let page_h = page_h_f.round() as i32;
                        self.tx_worker.send(RenderRequest {
                            page_idx: idx,
                            zoom: self.zoom,
                            page_y: page_y as f32,
                            page_height: page_h as f32,
                            window_height: height as f32,
                        }).ok();
                        requested.insert((idx, zoom_key));
                    }
                }
            }

            self.draw_splash_screen(&mut buffer, width, height);
            buffer.present().ok();
            self.surface = Some(surface);
            return;
        }

        if page_count == 0 {
            buffer.fill(bg);
            buffer.present().ok(); // present() consumes buffer
            self.surface = Some(surface);
            return;
        }

        // Compute horizontal layout (all pages same width, centered)
        let page_w_f = self.pages[0].width * self.zoom;
        let page_x0 = ((width as f32 - page_w_f) / 2.0).round() as i32;
        let left_w = page_x0.max(0) as usize; // width of left gray strip
        let page_right = (page_x0 + page_w_f.round() as i32).min(width as i32).max(0) as usize;
        let right_w = width.saturating_sub(page_right); // width of right gray strip

        // Binary search: skip pages entirely above viewport
        let first_visible = {
            let mut lo = 0usize;
            let mut hi = page_count;
            while lo < hi {
                let mid = (lo + hi) / 2;
                let bot = self.scroll_y + (self.pages[mid].top_y + self.pages[mid].height) * self.zoom;
                if bot < -100.0 { lo = mid + 1; } else { hi = mid; }
            }
            lo
        };

        // cursor_y tracks which screen row we've rendered up to
        let mut cursor_y: usize = 0;

        // Helper: fill rows [row_start, row_end) with bg (left+right strips) and `mid_fill` in center
        let fill_rows = |buf: &mut [u32], row_start: usize, row_end: usize, mid_fill: u32| {
            for row in row_start..row_end {
                let base = row * width;
                // left strip
                buf[base..base + left_w].fill(bg);
                // center
                if left_w < page_right {
                    buf[base + left_w..base + page_right].fill(mid_fill);
                }
                // right strip
                if page_right < width {
                    buf[base + page_right..base + width].fill(bg);
                }
            }
        };

        let mut next_non_visible = page_count;
        for page_idx in first_visible..page_count {
            let page = &self.pages[page_idx];
            let page_h_f = page.height * self.zoom;
            let page_y = (self.scroll_y + page.top_y * self.zoom).round() as i32;
            let page_h = page_h_f.round() as i32;

            // Send pre-fetch request if not cached
            {
                let image_cache = self.page_images.borrow();
                if !image_cache.contains_key(&page_idx) {
                    let zoom_key = (self.zoom * 1000.0) as u32;
                    let mut requested = self.requested_pages.borrow_mut();
                    if !requested.contains(&(page_idx, zoom_key)) {
                        self.tx_worker.send(RenderRequest {
                            page_idx,
                            zoom: self.zoom,
                            page_y: page_y as f32,
                            page_height: page_h as f32,
                            window_height: height as f32,
                        }).ok();
                        requested.insert((page_idx, zoom_key));
                    }
                }
            }

            // Stop once page is fully below viewport
            if page_y >= height as i32 {
                fill_rows(&mut buffer, cursor_y, height, bg);
                cursor_y = height;
                next_non_visible = page_idx;
                break;
            }

            // Fill gap between cursor_y and top of this page
            let gap_end = (page_y as usize).min(height);
            if gap_end > cursor_y {
                fill_rows(&mut buffer, cursor_y, gap_end, bg);
                cursor_y = gap_end;
            }

            // Row range of the page that is visible on screen
            let src_y_start = (-page_y).max(0) as usize; // first row of src pixmap to draw
            let dst_y_start = page_y.max(0) as usize;    // first screen row
            let dst_y_end = (page_y + page_h).min(height as i32).max(0) as usize;

            // Blit page from cache (or placeholder)
            let image_cache = self.page_images.borrow();
            if let Some(page_pixmap) = image_cache.get(&page_idx) {
                self.record_access_only(page_idx);
                let src_bytes = page_pixmap.data();
                let src_w = page_pixmap.width() as usize;

                // Clamp horizontal copy range
                let src_x_start = (-page_x0).max(0) as usize;
                let src_x_end = (page_w_f.round() as i32).min(width as i32 - page_x0).max(0) as usize;
                let dst_x_start = page_x0.max(0) as usize;
                let copy_w = if src_x_end > src_x_start { src_x_end - src_x_start } else { 0 };

                // Reinterpret source as u32: tiny-skia byte order [R,G,B,A] -> u32 LE = 0xAABBGGRR
                let src_u32: &[u32] = unsafe {
                    std::slice::from_raw_parts(
                        src_bytes.as_ptr() as *const u32,
                        src_bytes.len() / 4,
                    )
                };

                for dst_row in dst_y_start..dst_y_end {
                    let src_row = src_y_start + (dst_row - dst_y_start);
                    let base = dst_row * width;

                    // Left gray strip
                    if left_w > 0 { buffer[base..base + left_w].fill(bg); }

                    // Page pixels: swap R and B channels for softbuffer format
                    // src: 0xAABBGGRR -> dst: 0x00RRGGBB
                    if copy_w > 0 {
                        let src_start = src_row * src_w + src_x_start;
                        let src_slice = &src_u32[src_start..src_start + copy_w];
                        let dst_slice = &mut buffer[base + dst_x_start..base + dst_x_start + copy_w];
                        for (d, &s) in dst_slice.iter_mut().zip(src_slice.iter()) {
                            *d = ((s & 0x000000FF) << 16) | (s & 0x0000FF00) | ((s & 0x00FF0000) >> 16);
                        }
                    }

                    // Right gray strip
                    if right_w > 0 { buffer[base + page_right..base + width].fill(bg); }
                }
            } else {
                // White placeholder while page renders
                fill_rows(&mut buffer, dst_y_start, dst_y_end, white);
            }

            cursor_y = dst_y_end;
        }

        // Fill any remaining rows below last page
        if cursor_y < height {
            fill_rows(&mut buffer, cursor_y, height, bg);
        }

        // Send pre-fetch requests for pages around the viewport (Asymmetric margin)
        {
            let (preload_above, preload_below) = if self.scroll_down_direction {
                (1, 3)
            } else {
                (3, 1)
            };

            let zoom_key = (self.zoom * 1000.0) as u32;
            let image_cache = self.page_images.borrow();
            let mut requested = self.requested_pages.borrow_mut();

            // Pages above first_visible
            let start_above = first_visible.saturating_sub(preload_above);
            for idx in start_above..first_visible {
                if idx < page_count {
                    if image_cache.contains_key(&idx) {
                        self.record_access_only(idx);
                    } else if !requested.contains(&(idx, zoom_key)) {
                        let page = &self.pages[idx];
                        let page_h_f = page.height * self.zoom;
                        let page_y = (self.scroll_y + page.top_y * self.zoom).round() as i32;
                        let page_h = page_h_f.round() as i32;
                        self.tx_worker.send(RenderRequest {
                            page_idx: idx,
                            zoom: self.zoom,
                            page_y: page_y as f32,
                            page_height: page_h as f32,
                            window_height: height as f32,
                        }).ok();
                        requested.insert((idx, zoom_key));
                    }
                }
            }

            // Pages below next_non_visible
            let end_below = (next_non_visible + preload_below).min(page_count);
            for idx in next_non_visible..end_below {
                if idx < page_count {
                    if image_cache.contains_key(&idx) {
                        self.record_access_only(idx);
                    } else if !requested.contains(&(idx, zoom_key)) {
                        let page = &self.pages[idx];
                        let page_h_f = page.height * self.zoom;
                        let page_y = (self.scroll_y + page.top_y * self.zoom).round() as i32;
                        let page_h = page_h_f.round() as i32;
                        self.tx_worker.send(RenderRequest {
                            page_idx: idx,
                            zoom: self.zoom,
                            page_y: page_y as f32,
                            page_height: page_h as f32,
                            window_height: height as f32,
                        }).ok();
                        requested.insert((idx, zoom_key));
                    }
                }
            }
        }

        // Overlay UI (zoom buttons) - rendered with tiny-skia only when mouse is near it
        let overlay_width = 360.0f32;
        let overlay_height = 72.0f32;
        let overlay_x = width as f32 - overlay_width - 30.0;
        let overlay_y = height as f32 - overlay_height - 30.0;

        let hover_state = self.get_hover_state(self.mouse_pos.0, self.mouse_pos.1);
        let is_overlay_focused = hover_state > 0;

        if is_overlay_focused {
            // Render overlay to a small pixmap, then composite into buffer
            let ow = (overlay_width + 4.0) as u32;
            let oh = (overlay_height + 4.0) as u32;
            let ox_px = (overlay_x - 2.0).max(0.0) as usize;
            let oy_px = (overlay_y - 2.0).max(0.0) as usize;

            if let Some(mut ovl) = Pixmap::new(ow, oh) {
                let font = &self.default_font.clone();

                let mut bg_paint = Paint::default();
                bg_paint.set_color_rgba8(25, 25, 25, 220);

                let mut border_p = Paint::default();
                border_p.set_color_rgba8(100, 100, 100, 255);

                // Background + border
                if let Some(border_r) = Rect::from_xywh(0.0, 0.0, ow as f32, oh as f32) {
                    ovl.fill_rect(border_r, &border_p, Transform::identity(), None);
                }
                if let Some(inner_r) = Rect::from_xywh(1.0, 1.0, ow as f32 - 2.0, oh as f32 - 2.0) {
                    ovl.fill_rect(inner_r, &bg_paint, Transform::identity(), None);
                }

                let btn_y_local = 9.0f32;
                let btn_size = 54.0f32;

                let draw_btn = |ovl: &mut Pixmap, x: f32, label: &str, hovered: bool| {
                    let mut p = Paint::default();
                    p.set_color_rgba8(if hovered { 70 } else { 40 }, if hovered { 70 } else { 40 }, if hovered { 70 } else { 40 }, 255);
                    if let Some(r) = Rect::from_xywh(x, btn_y_local, btn_size, btn_size) {
                        ovl.fill_rect(r, &p, Transform::identity(), None);
                    }
                    let mut tp = Paint::default(); tp.set_color_rgba8(255, 255, 255, 255); tp.anti_alias = true;
                    let tw = self.measure_text_width(label, 26.0, font);
                    self.draw_text(ovl, label, x + (btn_size - tw) / 2.0, btn_y_local + 37.0, 26.0, font, &tp);
                };

                let minus_x = 15.0f32;
                let plus_x = 222.0f32;
                let reset_x = 291.0f32;
                draw_btn(&mut ovl, minus_x, "-", hover_state == 2);
                draw_btn(&mut ovl, plus_x, "+", hover_state == 3);
                draw_btn(&mut ovl, reset_x, "R", hover_state == 4);

                let current_fit_zoom = self.calculate_fit_zoom(width as u32, height as u32);
                let zoom_pct = format!("{:.0}%", (self.zoom / current_fit_zoom) * 100.0);
                let mut lp = Paint::default(); lp.set_color_rgba8(255, 255, 255, 255); lp.anti_alias = true;
                let lw = self.measure_text_width(&zoom_pct, 24.0, font);
                let lx = 69.0 + (153.0 - lw) / 2.0;
                self.draw_text(&mut ovl, &zoom_pct, lx, btn_y_local + 36.0, 24.0, font, &lp);

                // Composite overlay pixmap into buffer with R/B swap
                let ovl_data = ovl.data();
                let ovl_u32: &[u32] = unsafe {
                    std::slice::from_raw_parts(ovl_data.as_ptr() as *const u32, ovl_data.len() / 4)
                };
                for row in 0..oh as usize {
                    let dst_row = oy_px + row;
                    if dst_row >= height { break; }
                    for col in 0..ow as usize {
                        let dst_col = ox_px + col;
                        if dst_col >= width { break; }
                        let s = ovl_u32[row * ow as usize + col];
                        let a = (s >> 24) & 0xFF;
                        if a == 0 { continue; }
                        let dst_idx = dst_row * width + dst_col;
                        if a == 255 {
                            // Fully opaque: direct copy
                            buffer[dst_idx] = ((s & 0xFF) << 16) | (s & 0xFF00) | ((s >> 16) & 0xFF);
                        } else {
                            // Alpha blend
                            let inv_a = 255 - a;
                            let bg_val = buffer[dst_idx];
                            let sr = s & 0xFF;
                            let sg = (s >> 8) & 0xFF;
                            let sb = (s >> 16) & 0xFF;
                            let br = (bg_val >> 16) & 0xFF;
                            let bg_g = (bg_val >> 8) & 0xFF;
                            let bb = bg_val & 0xFF;
                            let nr = (sr * a + br * inv_a) / 255;
                            let ng = (sg * a + bg_g * inv_a) / 255;
                            let nb = (sb * a + bb * inv_a) / 255;
                            buffer[dst_idx] = (nr << 16) | (ng << 8) | nb;
                        }
                    }
                }
            }
        }

        if let Err(e) = buffer.present() { eprintln!("present: {}", e); }
        // buffer consumed by present(), surface borrow is released
        self.surface = Some(surface);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window = match event_loop.create_window(
                Window::default_attributes()
                    .with_title("UfReader - Pro Weight View")
                    .with_maximized(true)
            ) {
                Ok(w) => Rc::new(w),
                Err(e) => {
                    eprintln!("Failed to create window: {}", e);
                    return;
                }
            };
            let size = window.inner_size();
            self.window_size = size;
            let context = match Context::new(window.clone()) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to create softbuffer context: {}", e);
                    return;
                }
            };
            let surface = match Surface::new(&context, window.clone()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to create softbuffer surface: {}", e);
                    return;
                }
            };
            self.window = Some(window.clone());
            self.context = Some(context);
            self.surface = Some(surface);
            
            if let Some(icon) = load_window_icon() {
                window.set_window_icon(Some(icon));
            }
            
            if size.width > 0 && size.height > 0 {
                self.zoom = self.calculate_fit_zoom(size.width, size.height);
                        self.clear_cache();
                self.center_on_content(size.width, size.height);
                self.zoom_initialized = true;
            }
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.window_size = size;
                if let Some(surface) = self.surface.as_mut() {
                    if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
                        let _ = surface.resize(w, h);
                    }
                }
                if !self.zoom_initialized && size.width > 0 && size.height > 0 {
                    self.zoom = self.calculate_fit_zoom(size.width, size.height);
                    self.clear_cache();
                    self.center_on_content(size.width, size.height);
                    self.zoom_initialized = true;
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let new_mouse_pos = (position.x as f32, position.y as f32);
                let old_state = self.get_hover_state(self.mouse_pos.0, self.mouse_pos.1);
                let new_state = self.get_hover_state(new_mouse_pos.0, new_mouse_pos.1);
                self.mouse_pos = new_mouse_pos;
                if old_state != new_state {
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state: winit::event::ElementState::Pressed, button: winit::event::MouseButton::Left, .. } => {
                let hover = self.get_hover_state(self.mouse_pos.0, self.mouse_pos.1);
                if hover > 1 {
                    let old_zoom = self.zoom;
                    let new_zoom = match hover {
                        2 => (old_zoom / 1.1).clamp(0.1, 10.0),
                        3 => (old_zoom * 1.1).clamp(0.1, 10.0),
                        4 => self.calculate_fit_zoom(self.window_size.width, self.window_size.height),
                        _ => old_zoom,
                    };
                    if (new_zoom - old_zoom).abs() > 0.0001 {
                        let actual_factor = new_zoom / old_zoom;
                        self.zoom = new_zoom;
                        self.clear_cache();
                        let cx = self.window_size.width as f32 / 2.0;
                        let cy = self.window_size.height as f32 / 2.0;
                        self.scroll_x = self.scroll_x * actual_factor + cx * (1.0 - actual_factor);
                        self.scroll_y = self.scroll_y * actual_factor + cy * (1.0 - actual_factor);
                        self.target_scroll_x = self.scroll_x;
                        self.target_scroll_y = self.scroll_y;
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let total_h = self.pages.last().map(|p| p.top_y + p.height).unwrap_or(0.0) * self.zoom;
                let min_scroll = -(total_h - self.window_size.height as f32 + 100.0).max(0.0);
                let max_scroll = 100.0;

                match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => {
                        if self.modifiers.control_key() {
                            let old_zoom = self.zoom;
                            let factor = if y > 0.0 { 1.15 } else { 1.0 / 1.15 };
                            let new_zoom = (old_zoom * factor).clamp(0.1, 10.0);
                            let actual_factor = new_zoom / old_zoom;
                            self.zoom = new_zoom;
                            self.clear_cache();
                            let mx = self.mouse_pos.0;
                            let my = self.mouse_pos.1;
                            self.scroll_x = self.scroll_x * actual_factor + mx * (1.0 - actual_factor);
                            self.scroll_y = self.scroll_y * actual_factor + my * (1.0 - actual_factor);
                        } else {
                            // Standard mouse wheel - one notch = ~300px (~1/3 page), fast and natural
                            self.scroll_y = (self.scroll_y + y * 300.0).clamp(min_scroll, max_scroll);
                        }
                        self.target_scroll_x = self.scroll_x;
                        self.target_scroll_y = self.scroll_y;
                        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                    }
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        // Touchpad precision scroll - apply delta directly, 1:1 with finger
                        // Do NOT accumulate - just shift by the exact pixel amount reported
                        if pos.y.abs() > 0.1 {
                            if self.modifiers.control_key() {
                                let old_zoom = self.zoom;
                                let factor = if pos.y > 0.0 { 1.02 } else { 1.0 / 1.02 };
                                let new_zoom = (old_zoom * factor).clamp(0.1, 10.0);
                                let actual_factor = new_zoom / old_zoom;
                                self.zoom = new_zoom;
                                self.clear_cache();
                                let mx = self.mouse_pos.0;
                                let my = self.mouse_pos.1;
                                self.scroll_x = self.scroll_x * actual_factor + mx * (1.0 - actual_factor);
                                self.scroll_y = self.scroll_y * actual_factor + my * (1.0 - actual_factor);
                            } else {
                                self.scroll_y = (self.scroll_y + pos.y as f32).clamp(min_scroll, max_scroll);
                            }
                            self.target_scroll_x = self.scroll_x;
                            self.target_scroll_y = self.scroll_y;
                            if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => if let Some(window) = self.window.clone() { self.draw(&window); },
            WindowEvent::KeyboardInput { event: winit::event::KeyEvent { physical_key: winit::keyboard::PhysicalKey::Code(code), .. }, .. } => {
                match code {
                    winit::keyboard::KeyCode::ArrowUp => {
                        let total_h = self.pages.last().map(|p| p.top_y + p.height).unwrap_or(0.0) * self.zoom;
                        let min_scroll = -(total_h - self.window_size.height as f32 + 100.0).max(0.0);
                        self.scroll_y = (self.scroll_y + 200.0).clamp(min_scroll, 100.0);
                        self.target_scroll_y = self.scroll_y;
                    }
                    winit::keyboard::KeyCode::ArrowDown => {
                        let total_h = self.pages.last().map(|p| p.top_y + p.height).unwrap_or(0.0) * self.zoom;
                        let min_scroll = -(total_h - self.window_size.height as f32 + 100.0).max(0.0);
                        self.scroll_y = (self.scroll_y - 200.0).clamp(min_scroll, 100.0);
                        self.target_scroll_y = self.scroll_y;
                    }
                    winit::keyboard::KeyCode::Digit0 if self.modifiers.control_key() => {
                        let old_zoom = self.zoom;
                        let new_zoom = self.calculate_fit_zoom(self.window_size.width, self.window_size.height);
                        let actual_factor = new_zoom / old_zoom;
                        self.zoom = new_zoom;
                        self.clear_cache();
                        let cx = self.window_size.width as f32 / 2.0;
                        let cy = self.window_size.height as f32 / 2.0;
                        self.scroll_x = self.scroll_x * actual_factor + cx * (1.0 - actual_factor);
                        self.scroll_y = self.scroll_y * actual_factor + cy * (1.0 - actual_factor);
                        self.target_scroll_x = self.scroll_x;
                        self.target_scroll_y = self.scroll_y;
                    }
                    winit::keyboard::KeyCode::KeyC => {
                        self.center_on_content(self.window_size.width, self.window_size.height);
                    }
                    _ => (),
                }
                if let Some(window) = self.window.as_ref() { window.request_redraw(); }
            }
            _ => (),
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let mut got_any = false;
        while let Ok(msg) = self.rx_worker.try_recv() {
            match msg {
                WorkerMessage::PageRendered { page_idx, zoom, pixmap } => {
                    if (zoom - self.zoom).abs() < 0.001 {
                        self.page_images.borrow_mut().insert(page_idx, pixmap);
                        self.record_access_and_evict(page_idx);
                        let zoom_key = (self.zoom * 1000.0) as u32;
                        self.requested_pages.borrow_mut().remove(&(page_idx, zoom_key));
                        got_any = true;
                    }
                }
            }
        }
        if got_any {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
    }
}

fn get_font_path(key: &str) -> Option<String> {
    #[cfg(target_os = "windows")]
    let (folder, filename) = {
        let folder = "C:\\Windows\\Fonts\\";
        let filename = match key {
            "serif_regular" => "times.ttf",
            "serif_bold" => "timesbd.ttf",
            "serif_italic" => "timesi.ttf",
            "serif_bold_italic" => "timesbi.ttf",
            "sans_regular" => "arial.ttf",
            "sans_bold" => "arialbd.ttf",
            "sans_italic" => "ariali.ttf",
            "mono_regular" => "cour.ttf",
            _ => return None,
        };
        (folder, filename)
    };

    #[cfg(target_os = "macos")]
    let (folder, filename) = {
        let folder = "/System/Library/Fonts/Supplemental/";
        let filename = match key {
            "serif_regular" => "Times New Roman.ttf",
            "serif_bold" => "Times New Roman Bold.ttf",
            "serif_italic" => "Times New Roman Italic.ttf",
            "serif_bold_italic" => "Times New Roman Bold Italic.ttf",
            "sans_regular" => "Arial.ttf",
            "sans_bold" => "Arial Bold.ttf",
            "sans_italic" => "Arial Italic.ttf",
            "mono_regular" => "Courier New.ttf",
            _ => return None,
        };
        (folder, filename)
    };

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let (folder, filename) = {
        let folder = "/usr/share/fonts/truetype/dejavu/";
        let filename = match key {
            "serif_regular" => "DejaVuSerif.ttf",
            "serif_bold" => "DejaVuSerif-Bold.ttf",
            "serif_italic" => "DejaVuSerif-Italic.ttf",
            "serif_bold_italic" => "DejaVuSerif-BoldItalic.ttf",
            "sans_regular" => "DejaVuSans.ttf",
            "sans_bold" => "DejaVuSans-Bold.ttf",
            "sans_italic" => "DejaVuSans-Oblique.ttf",
            "mono_regular" => "DejaVuSansMono.ttf",
            _ => return None,
        };
        (folder, filename)
    };

    Some(format!("{}{}", folder, filename))
}

fn map_font_name(basefont: &str) -> &'static str {
    let bf = basefont.to_lowercase();
    let is_bold = bf.contains("bold") || bf.contains("black") || bf.contains("w7") || bf.contains("negoti");
    let is_italic = bf.contains("italic") || bf.contains("oblique") || bf.contains("kursiv");
    
    if bf.contains("sans") || bf.contains("helvetica") || bf.contains("arial") {
        if is_bold { "sans_bold" }
        else if is_italic { "sans_italic" }
        else { "sans_regular" }
    } else if bf.contains("mono") || bf.contains("courier") || bf.contains("consolas") {
        "mono_regular"
    } else {
        if is_bold && is_italic { "serif_bold_italic" }
        else if is_bold { "serif_bold" }
        else if is_italic { "serif_italic" }
        else { "serif_regular" }
    }
}

pub struct Gui {
    pub pdf_path: String,
    pub pages: Vec<PageInfo>,
}

impl Gui {
    pub fn new(pdf_path: String, pages: Vec<PageInfo>) -> Self {
        Self { pdf_path, pages }
    }

    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let event_loop = EventLoop::new()?;
        let proxy = event_loop.create_proxy();
        
        let font_keys = [
            "serif_regular", "serif_bold", "serif_italic", "serif_bold_italic",
            "sans_regular", "sans_bold", "sans_italic", "mono_regular"
        ];
        
        let mut loaded_fonts = std::collections::HashMap::new();
        let mut default_font = None;

        for key in &font_keys {
            if let Some(path) = get_font_path(key) {
                if let Ok(data) = std::fs::read(&path) {
                    if let Ok(f) = ab_glyph::FontVec::try_from_vec(data) {
                        let f_arc = Arc::new(f);
                        if default_font.is_none() {
                            default_font = Some(f_arc.clone());
                        }
                        loaded_fonts.insert(key.to_string(), f_arc);
                    }
                }
            }
        }

        if loaded_fonts.is_empty() {
            let fallback_paths = [
                "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
                "/usr/share/fonts/dejavu/DejaVuSerif.ttf",
                "C:\\Windows\\Fonts\\times.ttf",
                "C:\\Windows\\Fonts\\georgia.ttf",
                "C:\\Windows\\Fonts\\arial.ttf",
                "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
            ];
            for path in &fallback_paths {
                if let Ok(data) = std::fs::read(path) {
                    if let Ok(f) = ab_glyph::FontVec::try_from_vec(data) {
                        let f_arc = Arc::new(f);
                        loaded_fonts.insert("serif_regular".to_string(), f_arc.clone());
                        default_font = Some(f_arc);
                        break;
                    }
                }
            }
        }

        let default_font = match default_font {
            Some(f) => f,
            None => {
                return Err("Could not load any system font!".into());
            }
        };

        // Create channels for background worker
        let (tx_worker, rx_request) = std::sync::mpsc::channel();
        let (tx_response, rx_worker) = std::sync::mpsc::channel();

        // Spawn background worker thread
        let pdf_path_clone = self.pdf_path.clone();
        let worker_fonts = loaded_fonts.clone();
        let worker_default_font = default_font.clone();
        std::thread::spawn(move || {
            run_worker_thread(
                pdf_path_clone,
                worker_fonts,
                worker_default_font,
                rx_request,
                tx_response,
                proxy,
            );
        });
        
        let mut app = App {
            pages: self.pages,
            window: None,
            context: None,
            surface: None,
            page_images: RefCell::new(std::collections::HashMap::new()),
            requested_pages: RefCell::new(std::collections::HashSet::new()),
            page_access_order: RefCell::new(std::collections::VecDeque::new()),
            tx_worker,
            rx_worker,
            scroll_x: 0.0,
            scroll_y: 0.0,
            target_scroll_x: 0.0,
            target_scroll_y: 0.0,
            last_scroll_y: 0.0,
            scroll_down_direction: true,
            zoom: 1.0,
            modifiers: winit::keyboard::ModifiersState::default(),
            mouse_pos: (0.0, 0.0),
            window_size: winit::dpi::PhysicalSize::new(0, 0),
            zoom_initialized: false,
            default_font,
            logo_pixmap: Pixmap::load_png("logo.png").ok(),
        };
        event_loop.run_app(&mut app)?;
        Ok(())
    }
}

fn run_worker_thread(
    pdf_path: String,
    fonts: std::collections::HashMap<String, Arc<FontVec>>,
    default_font: Arc<FontVec>,
    rx_request: std::sync::mpsc::Receiver<RenderRequest>,
    tx_response: std::sync::mpsc::Sender<WorkerMessage>,
    proxy: winit::event_loop::EventLoopProxy<()>,
) {
    let mut parser = match crate::parser::Parser::new(&pdf_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Worker failed to open PDF: {}", e);
            return;
        }
    };
    if let Err(e) = parser.parse_metadata() {
        eprintln!("Worker failed to parse PDF metadata: {}", e);
        return;
    }

    let pdf_fonts = parser.find_fonts();
    let mut font_encodings = std::collections::HashMap::new();
    let mut font_widths = std::collections::HashMap::new();
    let mut font_names = std::collections::HashMap::new();
    
    for (id, dict) in &pdf_fonts {
        let font_name = if let Some(PdfObject::Name(n)) = dict.get("Name") {
            n.clone()
        } else {
            format!("F{}", id)
        };

        let base_font = if let Some(PdfObject::Name(bf)) = dict.get("BaseFont") {
            bf.clone()
        } else {
            "Serif".to_string()
        };
        font_names.insert(font_name.clone(), base_font);

        let mut encoding = crate::interpreter::PdfEncoding::WinAnsi;

        let widths_obj = dict.get("Widths").cloned();
        let resolved_widths = if let Some(ref_obj) = widths_obj {
            parser.resolve_reference(&ref_obj).ok()
        } else {
            None
        };

        if let Some(PdfObject::Array(w)) = resolved_widths {
            let first_char = if let Some(PdfObject::Integer(fc)) = dict.get("FirstChar") { *fc as u32 } else { 0 };
            let mut widths_map = std::collections::HashMap::new();
            for (i, width_val) in w.iter().enumerate() {
                let width = match width_val {
                    PdfObject::Integer(v) => *v as f32,
                    PdfObject::Real(v) => *v as f32,
                    _ => 0.0,
                };
                if width != 0.0 {
                    widths_map.insert(first_char + i as u32, width);
                }
            }
            font_widths.insert(font_name.clone(), widths_map);
        }

        if let Some(enc_obj) = dict.get("Encoding") {
            match enc_obj {
                PdfObject::Name(n) => {
                    if n == "MacRomanEncoding" { encoding = crate::interpreter::PdfEncoding::MacRoman; }
                    else if n == "WinAnsiEncoding" { encoding = crate::interpreter::PdfEncoding::WinAnsi; }
                }
                PdfObject::Dictionary(enc_dict) => {
                    if let Some(PdfObject::Array(diffs)) = enc_dict.get("Differences") {
                        let mut map = std::collections::HashMap::new();
                        let mut current_code = 0;
                        for obj in diffs {
                            match obj {
                                PdfObject::Integer(code) => { current_code = *code as u32; }
                                PdfObject::Name(glyph_name) => {
                                    let c = match glyph_name.as_str() {
                                        "aacute" => 'á', "eacute" => 'é', "iacute" => 'í', "oacute" => 'ó', "uacute" => 'ú',
                                        "Aacute" => 'Á', "Eacute" => 'É', "Iacute" => 'Í', "Oacute" => 'Ó', "Uacute" => 'Ú',
                                        "ntilde" => 'ñ', "Ntilde" => 'Ñ', "udieresis" => 'ü', "Udieresis" => 'Ü',
                                        "questiondown" => '¿', "exclamdown" => '¡',
                                        "space" => ' ', _ => ' ',
                                    };
                                    if c != ' ' { map.insert(current_code, c); }
                                    current_code += 1;
                                }
                                _ => {}
                            }
                        }
                        if !map.is_empty() {
                            encoding = crate::interpreter::PdfEncoding::Custom(map);
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some(to_unicode_ref) = dict.get("ToUnicode") {
            if let Ok(m) = parser.parse_cmap(to_unicode_ref) {
                encoding = crate::interpreter::PdfEncoding::Custom(m);
            }
        }
        
        font_encodings.insert(font_name, encoding);
    }
    
    // Setup fallback widths
    let mut fallback_char_widths = std::collections::HashMap::new();
    let units_per_em = default_font.units_per_em().unwrap_or(1000.0);
    for i in 32..=255u32 {
        if let Some(c) = std::char::from_u32(i) {
            let glyph_id = default_font.glyph_id(c);
            let advance = default_font.h_advance_unscaled(glyph_id);
            fallback_char_widths.insert(c, (advance / units_per_em) * 1000.0);
        }
    }

    let interpreter = crate::interpreter::Interpreter::new(font_encodings, font_widths, fallback_char_widths, font_names);
    
    // Worker cache for page commands and compiled paths
    let mut page_commands_cache = std::collections::HashMap::new();
    let mut page_paths_cache: std::collections::HashMap<(usize, &'static str), Option<Path>> = std::collections::HashMap::new();

    let font_keys = [
        "serif_regular", "serif_bold", "serif_italic", "serif_bold_italic",
        "sans_regular", "sans_bold", "sans_italic", "mono_regular"
    ];

    let select_font_and_key = |basefont: &str| -> (&Arc<FontVec>, &'static str) {
        let key = map_font_name(basefont);
        if fonts.contains_key(key) {
            (&fonts[key], key)
        } else {
            (&default_font, "default")
        }
    };

    let mut local_queue: Vec<RenderRequest> = Vec::new();

    loop {
        if local_queue.is_empty() {
            match rx_request.recv() {
                Ok(req) => local_queue.push(req),
                Err(_) => break, // Channel disconnected
            }
        }

        // Drain any pending requests in the channel
        while let Ok(req) = rx_request.try_recv() {
            local_queue.push(req);
        }

        // Filter by the latest requested zoom level
        if let Some(latest) = local_queue.last() {
            let latest_zoom = latest.zoom;
            local_queue.retain(|req| (req.zoom - latest_zoom).abs() < 0.001);
        }

        // Deduplicate: keep only the latest request for each page_idx
        let mut unique = std::collections::HashMap::new();
        for req in local_queue.drain(..) {
            unique.insert(req.page_idx, req);
        }
        local_queue = unique.into_values().collect();

        // Sort requests: closest to viewport center first (distance to viewport boundary)
        local_queue.sort_by(|a, b| {
            let dist_a = get_distance_to_viewport(a.page_y, a.page_height, a.window_height);
            let dist_b = get_distance_to_viewport(b.page_y, b.page_height, b.window_height);
            dist_a.partial_cmp(&dist_b).unwrap_or(std::cmp::Ordering::Equal)
        });

        if local_queue.is_empty() {
            continue;
        }

        // Process the highest priority request (index 0)
        let request = local_queue.remove(0);
        let page_idx = request.page_idx;
        let zoom = request.zoom;

        // 1. Get or parse page commands
        let commands = page_commands_cache.entry(page_idx).or_insert_with(|| {
            let page_rect = parser.get_page_rect(page_idx).unwrap_or(crate::parser::PageRect {
                x: 0.0, y: 0.0, width: 595.0, height: 842.0
            });
            match parser.get_page_content(page_idx) {
                Ok(content) => interpreter.process(page_idx, &content, page_rect),
                Err(_) => Vec::new(),
            }
        });

        let page_rect = parser.get_page_rect(page_idx).unwrap_or(crate::parser::PageRect {
            x: 0.0, y: 0.0, width: 595.0, height: 842.0
        });

        // 2. Build font paths for this page if not cached
        for &font_key in &font_keys {
            let has_path = page_paths_cache.contains_key(&(page_idx, font_key));
            if !has_path {
                let mut path_builder = PathBuilder::new();
                let mut has_glyphs = false;

                for cmd in commands.iter() {
                    let DrawCommand::Text { chars, local_y, size, font_name, .. } = cmd;
                    let (_, cmd_font_key) = select_font_and_key(font_name);
                    if cmd_font_key != font_key { continue; }

                    let (font, _) = select_font_and_key(font_name);
                    let scale_factor = *size / font.units_per_em().unwrap_or(1000.0);
                    if !scale_factor.is_finite() { continue; }

                    for (c, x, expected_w) in chars {
                        let char_x = *x;
                        let local_screen_y = page_rect.height - *local_y;
                        let char_y = local_screen_y;
                        if !char_x.is_finite() || !char_y.is_finite() { continue; }

                        let glyph_id = font.glyph_id(*c);
                        if let Some(outline) = font.outline(glyph_id) {
                            has_glyphs = true;
                            let actual_w = font.h_advance_unscaled(glyph_id) * scale_factor;
                            let expected_w_val = *expected_w;
                            let h_squeeze = if actual_w > 0.0 && expected_w_val > 0.0 {
                                 expected_w_val / actual_w
                            } else { 1.0 };
                            let h_squeeze = h_squeeze.clamp(0.4, 2.5);
                            if !h_squeeze.is_finite() { continue; }

                            let mut last_point: Option<ab_glyph::Point> = None;
                            for curve in outline.curves {
                                let start_p = match curve {
                                    ab_glyph::OutlineCurve::Line(p1, _) => p1,
                                    ab_glyph::OutlineCurve::Quad(p1, _, _) => p1,
                                    ab_glyph::OutlineCurve::Cubic(p1, _, _, _) => p1,
                                };

                                let is_new_contour = match last_point {
                                    Some(lp) => (start_p.x - lp.x).abs() > 0.001 || (start_p.y - lp.y).abs() > 0.001,
                                    None => true,
                                };

                                if is_new_contour {
                                    path_builder.move_to(
                                        char_x + start_p.x * scale_factor * h_squeeze,
                                        char_y - start_p.y * scale_factor,
                                    );
                                }

                                match curve {
                                    ab_glyph::OutlineCurve::Line(_, p2) => {
                                        path_builder.line_to(
                                            char_x + p2.x * scale_factor * h_squeeze,
                                            char_y - p2.y * scale_factor,
                                        );
                                        last_point = Some(p2);
                                    }
                                    ab_glyph::OutlineCurve::Quad(_, p2, p3) => {
                                        path_builder.quad_to(
                                            char_x + p2.x * scale_factor * h_squeeze,
                                            char_y - p2.y * scale_factor,
                                            char_x + p3.x * scale_factor * h_squeeze,
                                            char_y - p3.y * scale_factor,
                                        );
                                        last_point = Some(p3);
                                    }
                                    ab_glyph::OutlineCurve::Cubic(_, p2, p3, p4) => {
                                        path_builder.cubic_to(
                                            char_x + p2.x * scale_factor * h_squeeze,
                                            char_y - p2.y * scale_factor,
                                            char_x + p3.x * scale_factor * h_squeeze,
                                            char_y - p3.y * scale_factor,
                                            char_x + p4.x * scale_factor * h_squeeze,
                                            char_y - p4.y * scale_factor,
                                        );
                                        last_point = Some(p4);
                                    }
                                }
                            }
                        }
                    }
                }

                let path_opt = if has_glyphs {
                    path_builder.finish()
                } else {
                    None
                };
                page_paths_cache.insert((page_idx, font_key), path_opt);
            }
        }

        // 3. Render page paths to target zoomed Pixmap
        let page_w = page_rect.width * zoom;
        let page_h = page_rect.height * zoom;
        let w = page_w.round() as u32;
        let h = page_h.round() as u32;

        if w > 0 && h > 0 {
            if let Some(mut page_pixmap) = Pixmap::new(w, h) {
                page_pixmap.fill(Color::from_rgba8(255, 255, 255, 255));

                let mut text_paint = Paint::default();
                text_paint.set_color_rgba8(0, 0, 0, 255);
                text_paint.anti_alias = true;

                for &font_key in &font_keys {
                    if let Some(Some(p)) = page_paths_cache.get(&(page_idx, font_key)) {
                        let transform = Transform::from_scale(zoom, zoom);
                        page_pixmap.fill_path(p, &text_paint, FillRule::Winding, transform, None);
                    }
                }

                // Send pixmap back
                tx_response.send(WorkerMessage::PageRendered {
                    page_idx,
                    zoom,
                    pixmap: page_pixmap,
                }).ok();

                // Wake up GUI event loop
                proxy.send_event(()).ok();
            }
        }
    }
}

fn get_distance_to_viewport(page_y: f32, page_height: f32, window_height: f32) -> f32 {
    if page_y + page_height < 0.0 {
        -(page_y + page_height)
    } else if page_y > window_height {
        page_y - window_height
    } else {
        0.0
    }
}

