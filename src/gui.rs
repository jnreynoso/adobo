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
use std::sync::atomic::{AtomicUsize, Ordering};
use ab_glyph::{FontVec, Font};
use crate::interpreter::DrawCommand;
use crate::object::PdfObject;


#[derive(Debug, Clone)]
pub struct PageInfo {
    pub width: f32,
    pub height: f32,
    pub top_y: f32,
    pub center_x_offset: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayoutMode {
    SinglePage,
    TwoPage,
    Continuous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RenderPriority {
    High,
    Low,
}

pub struct RenderRequest {
    pub epoch: usize,
    pub priority: RenderPriority,
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
    PageRenderAborted {
        page_idx: usize,
        zoom: f32,
    },
}

pub struct CachedPage {
    pub pixmap: Pixmap,
    pub zoom: f32,
}

struct App {
    pages: Vec<PageInfo>,
    window: Option<Rc<Window>>,
    context: Option<Context<Rc<Window>>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    page_images: RefCell<std::collections::HashMap<usize, CachedPage>>,
    requested_pages: RefCell<std::collections::HashSet<(usize, u32)>>,
    page_access_order: RefCell<std::collections::VecDeque<usize>>,
    tx_worker: std::sync::mpsc::Sender<RenderRequest>,
    rx_worker: std::sync::mpsc::Receiver<WorkerMessage>,
    render_epoch: Arc<AtomicUsize>,
    scroll_x: f32,
    scroll_y: f32,
    target_scroll_x: f32,
    target_scroll_y: f32,
    last_scroll_y: f32,
    scroll_down_direction: bool,
    zoom: f32,
    rendered_zoom: f32,
    last_zoom_change_time: std::time::Instant,
    modifiers: winit::keyboard::ModifiersState,
    mouse_pos: (f32, f32),
    window_size: winit::dpi::PhysicalSize<u32>,
    zoom_initialized: bool,
    default_font: Arc<FontVec>,
    logo_pixmap: Option<Pixmap>,
    layout_mode: LayoutMode,
    left_menu_open: bool,
    page_input_active: bool,
    page_input_text: String,
}

fn load_window_icon() -> Option<winit::window::Icon> {
    if let Ok(pixmap) = Pixmap::load_png("assets/logo.png") {
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
    fn recalculate_layout(&mut self) {
        let mut current_y = 0.0;
        let gap = 20.0;
        let mut i = 0;
        while i < self.pages.len() {
            if self.layout_mode == LayoutMode::Continuous || self.layout_mode == LayoutMode::SinglePage {
                self.pages[i].top_y = current_y;
                self.pages[i].center_x_offset = 0.0;
                current_y += self.pages[i].height + gap;
                i += 1;
            } else if self.layout_mode == LayoutMode::TwoPage {
                if i == 0 {
                    // Title page alone in TwoPage mode? Usually yes, but let's just make page 1 and 2 side by side.
                    // Wait, standard TwoPage: page 0 alone, page 1 and 2 side by side.
                    // But we can just make them pairs from the start for simplicity, or 0 alone.
                    // Let's pair them directly for simplicity.
                }
                if i + 1 < self.pages.len() {
                    let h1 = self.pages[i].height;
                    let h2 = self.pages[i+1].height;
                    let max_h = h1.max(h2);
                    let w1 = self.pages[i].width;
                    let w2 = self.pages[i+1].width;
                    
                    self.pages[i].top_y = current_y;
                    self.pages[i].center_x_offset = -w1 / 2.0 - 10.0;
                    self.pages[i+1].top_y = current_y;
                    self.pages[i+1].center_x_offset = w2 / 2.0 + 10.0;
                    
                    current_y += max_h + gap;
                    i += 2;
                } else {
                    self.pages[i].top_y = current_y;
                    self.pages[i].center_x_offset = 0.0;
                    current_y += self.pages[i].height + gap;
                    i += 1;
                }
            }
        }
        let total_h = self.pages.last().map(|p| p.top_y + p.height).unwrap_or(0.0) * self.zoom;
        let min_scroll = -(total_h - self.window_size.height as f32 + 100.0).max(0.0);
        self.scroll_y = self.scroll_y.clamp(min_scroll, 100.0);
        self.target_scroll_y = self.scroll_y;
        if let Some(w) = self.window.as_ref() { w.request_redraw(); }
    }

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
        let margin = 40.0;
        let base_w = self.pages[0].width;
        let w = if self.layout_mode == LayoutMode::TwoPage { base_w * 2.0 + 20.0 } else { base_w };
        let zoom_w = (width as f32 - margin) / w;
        let zoom_h = (height as f32 - margin) / self.pages[0].height;
        zoom_w.min(zoom_h).clamp(0.1, 10.0)
    }

    fn calculate_fit_width_zoom(&self, width: u32) -> f32 {
        if self.pages.is_empty() || width == 0 { return 1.0; }
        let margin = 40.0;
        let base_w = self.pages[0].width;
        let w = if self.layout_mode == LayoutMode::TwoPage { base_w * 2.0 + 20.0 } else { base_w };
        ((width as f32 - margin) / w).clamp(0.1, 10.0)
    }

    fn calculate_fit_height_zoom(&self, height: u32) -> f32 {
        if self.pages.is_empty() || height == 0 { return 1.0; }
        let margin = 40.0;
        ((height as f32 - margin) / self.pages[0].height).clamp(0.1, 10.0)
    }

    fn set_target_zoom(&mut self, new_zoom: f32) {
        let old_zoom = self.zoom;
        if (new_zoom - old_zoom).abs() > 0.0001 {
            let actual_factor = new_zoom / old_zoom;
            self.zoom = new_zoom;
            self.last_zoom_change_time = std::time::Instant::now();
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

    fn get_current_page_idx(&self) -> usize {
        if self.pages.is_empty() {
            return 0;
        }
        let viewport_center = self.window_size.height as f32 / 2.0;
        for (idx, page) in self.pages.iter().enumerate() {
            let top = self.scroll_y + page.top_y * self.zoom;
            let bottom = top + page.height * self.zoom;
            if viewport_center >= top && viewport_center <= bottom {
                return idx;
            }
        }
        for (idx, page) in self.pages.iter().enumerate() {
            let bottom = self.scroll_y + (page.top_y + page.height) * self.zoom;
            if bottom > 0.0 {
                return idx;
            }
        }
        0
    }

    fn jump_to_page(&mut self, idx: usize) {
        if idx >= self.pages.len() { return; }
        
        self.render_epoch.fetch_add(1, Ordering::SeqCst);
        self.requested_pages.borrow_mut().clear();
        
        let page = &self.pages[idx];
        let mut target_y = -page.top_y * self.zoom;
        
        let page_h_zoomed = page.height * self.zoom;
        if page_h_zoomed < self.window_size.height as f32 {
            target_y += (self.window_size.height as f32 - page_h_zoomed) / 2.0;
        } else {
            target_y += 20.0;
        }

        let total_h = self.pages.last().map(|p| p.top_y + p.height).unwrap_or(0.0) * self.zoom;
        let min_scroll = -(total_h - self.window_size.height as f32 + 100.0).max(0.0);
        self.scroll_y = target_y.clamp(min_scroll, 100.0);
        self.target_scroll_y = self.scroll_y;
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
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
        let title = "Adobo";
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

        let menu_btn_x = 30.0;
        let menu_btn_y = height - 100.0 - 30.0;
        let menu_btn_w = 84.0;
        let menu_btn_h = 100.0;

        if self.left_menu_open {
            let menu_w = 364.0;
            let menu_h = 448.0;
            let menu_x = 30.0;
            let menu_y = height - 100.0 - 30.0 - menu_h - 10.0;

            if mx >= menu_x && mx <= menu_x + menu_w && my >= menu_y && my <= menu_y + menu_h {
                let item_h = menu_h / 8.0;
                let idx = ((my - menu_y) / item_h).floor() as u8;
                return 10 + idx.clamp(0, 7);
            }
        }

        if mx >= menu_btn_x && mx <= menu_btn_x + menu_btn_w && my >= menu_btn_y && my <= menu_btn_y + menu_btn_h {
            return 9; // Menu toggle button
        }

        // Check bottom-center pagination overlay
        let pag_overlay_width = 372.0;
        let pag_overlay_height = 100.0;
        let pag_overlay_x = (width - pag_overlay_width) / 2.0;
        let pag_overlay_y = height - pag_overlay_height - 30.0;

        let is_pag_focused = mx >= pag_overlay_x - 30.0
            && mx <= pag_overlay_x + pag_overlay_width + 30.0
            && my >= pag_overlay_y - 30.0
            && my <= pag_overlay_y + pag_overlay_height + 30.0;

        if is_pag_focused {
            let btn_y_local = 12.0;
            let btn_size = 76.0;

            // Prev button: starts at 12.0
            let prev_x = pag_overlay_x + 12.0;
            if mx >= prev_x && mx <= prev_x + btn_size 
                && my >= pag_overlay_y + btn_y_local && my <= pag_overlay_y + btn_y_local + btn_size 
            {
                return 21;
            }

            // Next button: starts at 284.0
            let next_x = pag_overlay_x + 284.0;
            if mx >= next_x && mx <= next_x + btn_size 
                && my >= pag_overlay_y + btn_y_local && my <= pag_overlay_y + btn_y_local + btn_size 
            {
                return 22;
            }

            // Editable text area: starts at 100.0 to 180.0
            let edit_x = pag_overlay_x + 100.0;
            let edit_w = 80.0;
            if mx >= edit_x && mx <= edit_x + edit_w 
                && my >= pag_overlay_y + btn_y_local && my <= pag_overlay_y + btn_y_local + btn_size 
            {
                return 23;
            }

            return 20; // Pagination overlay focused but no button hovered
        }

        let overlay_width = 504.0;
        let overlay_height = 100.0;
        let overlay_x = width - overlay_width - 30.0;
        let overlay_y = height - overlay_height - 30.0;

        let is_overlay_focused = mx >= overlay_x - 30.0 
            && mx <= width + 10.0 
            && my >= overlay_y - 30.0 
            && my <= height + 10.0;

        if !is_overlay_focused {
            return 0;
        }

        let btn_y = overlay_y + 12.0;
        let btn_size = 76.0;

        // Minus button
        let minus_x = overlay_x + 20.0;
        if mx >= minus_x && mx <= minus_x + btn_size && my >= btn_y && my <= btn_y + btn_size {
            return 2;
        }

        // Plus button
        let plus_x = overlay_x + 306.0;
        if mx >= plus_x && mx <= plus_x + btn_size && my >= btn_y && my <= btn_y + btn_size {
            return 3;
        }

        // Reset button
        let reset_x = overlay_x + 408.0;
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
        let epoch = self.render_epoch.load(Ordering::SeqCst);

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
                            epoch,
                            priority: RenderPriority::High,
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
                            epoch,
                            priority: RenderPriority::Low,
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
                            epoch,
                            priority: RenderPriority::Low,
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

        buffer.fill(bg);

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

        let mut next_non_visible = page_count;
        for page_idx in first_visible..page_count {
            let page = &self.pages[page_idx];
            let page_h_f = page.height * self.zoom;
            let page_y = (self.scroll_y + page.top_y * self.zoom).round() as i32;
            let page_h = page_h_f.round() as i32;

            let page_w_f = page.width * self.zoom;
            let page_x0 = ((width as f32 / 2.0) + (page.center_x_offset * self.zoom) - (page_w_f / 2.0)).round() as i32;

            // Send pre-fetch request if not cached
            if (self.zoom - self.rendered_zoom).abs() < 0.001 {
                let image_cache = self.page_images.borrow();
                let needs_render = match image_cache.get(&page_idx) {
                    Some(cached) => (cached.zoom - self.zoom).abs() > 0.001,
                    None => true,
                };
                if needs_render {
                    let zoom_key = (self.zoom * 1000.0) as u32;
                    let mut requested = self.requested_pages.borrow_mut();
                    if !requested.contains(&(page_idx, zoom_key)) {
                        self.tx_worker.send(RenderRequest {
                            epoch,
                            priority: RenderPriority::High,
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

            if page_y >= height as i32 {
                next_non_visible = page_idx;
                break;
            }

            let src_y_start = (-page_y).max(0) as usize;
            let dst_y_start = page_y.max(0) as usize;
            let dst_y_end = (page_y + page_h).min(height as i32).max(0) as usize;

            let src_x_start_page = (-page_x0).max(0) as usize;
            let dst_x_start = page_x0.max(0) as usize;
            let dst_x_end = (page_x0 + page_w_f.round() as i32).min(width as i32).max(0) as usize;
            let copy_w = if dst_x_end > dst_x_start { dst_x_end - dst_x_start } else { 0 };

            let image_cache = self.page_images.borrow();
            if let Some(cached_page) = image_cache.get(&page_idx) {
                self.record_access_only(page_idx);
                if (cached_page.zoom - self.zoom).abs() < 0.001 {
                    // Fast path
                    let src_bytes = cached_page.pixmap.data();
                    let src_w = cached_page.pixmap.width() as usize;

                    let src_u32: &[u32] = unsafe {
                        std::slice::from_raw_parts(src_bytes.as_ptr() as *const u32, src_bytes.len() / 4)
                    };

                    for dst_row in dst_y_start..dst_y_end {
                        let src_row = src_y_start + (dst_row - dst_y_start);
                        let base = dst_row * width;
                        
                        if copy_w > 0 {
                            let src_start = src_row * src_w + src_x_start_page;
                            let src_slice = &src_u32[src_start..src_start + copy_w];
                            let dst_slice = &mut buffer[base + dst_x_start..base + dst_x_start + copy_w];
                            for (d, &s) in dst_slice.iter_mut().zip(src_slice.iter()) {
                                *d = ((s & 0x000000FF) << 16) | (s & 0x0000FF00) | ((s & 0x00FF0000) >> 16);
                            }
                        }
                    }
                } else {
                    // Slow path
                    let inv_scale = cached_page.zoom / self.zoom;
                    let src_bytes = cached_page.pixmap.data();
                    let src_w = cached_page.pixmap.width() as usize;
                    let src_h = cached_page.pixmap.height() as usize;

                    if src_w > 0 && src_h > 0 {
                        let src_u32: &[u32] = unsafe {
                            std::slice::from_raw_parts(src_bytes.as_ptr() as *const u32, src_bytes.len() / 4)
                        };
                        let src_w_sub = src_w.saturating_sub(1);
                        let src_h_sub = src_h.saturating_sub(1);

                        for dst_row in dst_y_start..dst_y_end {
                            let base = dst_row * width;
                            if copy_w > 0 {
                                let y_in_page = (src_y_start + (dst_row - dst_y_start)) as f32;
                                let src_y = ((y_in_page * inv_scale) as usize).min(src_h_sub);
                                let src_row_offset = src_y * src_w;
                                
                                let dst_slice = &mut buffer[base + dst_x_start..base + dst_x_start + copy_w];
                                let mut src_x_frac = src_x_start_page as f32 * inv_scale;
                                
                                for d in dst_slice.iter_mut() {
                                    let src_x = (src_x_frac as usize).min(src_w_sub);
                                    let s = src_u32[src_row_offset + src_x];
                                    *d = ((s & 0x000000FF) << 16) | (s & 0x0000FF00) | ((s & 0x00FF0000) >> 16);
                                    src_x_frac += inv_scale;
                                }
                            }
                        }
                    } else {
                        for dst_row in dst_y_start..dst_y_end {
                            let base = dst_row * width;
                            if copy_w > 0 { buffer[base + dst_x_start..base + dst_x_start + copy_w].fill(white); }
                        }
                    }
                }
            } else {
                for dst_row in dst_y_start..dst_y_end {
                    let base = dst_row * width;
                    if copy_w > 0 { buffer[base + dst_x_start..base + dst_x_start + copy_w].fill(white); }
                }
            }
        }

        // Send pre-fetch requests for pages around the viewport (Asymmetric margin)
        if (self.zoom - self.rendered_zoom).abs() < 0.001 {
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
                    let is_cached_at_zoom = match image_cache.get(&idx) {
                        Some(cached) => (cached.zoom - self.zoom).abs() < 0.001,
                        None => false,
                    };
                    if is_cached_at_zoom {
                        self.record_access_only(idx);
                    } else if !requested.contains(&(idx, zoom_key)) {
                        let page = &self.pages[idx];
                        let page_h_f = page.height * self.zoom;
                        let page_y = (self.scroll_y + page.top_y * self.zoom).round() as i32;
                        let page_h = page_h_f.round() as i32;
                        self.tx_worker.send(RenderRequest {
                            epoch,
                            priority: RenderPriority::Low,
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
                    let is_cached_at_zoom = match image_cache.get(&idx) {
                        Some(cached) => (cached.zoom - self.zoom).abs() < 0.001,
                        None => false,
                    };
                    if is_cached_at_zoom {
                        self.record_access_only(idx);
                    } else if !requested.contains(&(idx, zoom_key)) {
                        let page = &self.pages[idx];
                        let page_h_f = page.height * self.zoom;
                        let page_y = (self.scroll_y + page.top_y * self.zoom).round() as i32;
                        let page_h = page_h_f.round() as i32;
                        self.tx_worker.send(RenderRequest {
                            epoch,
                            priority: RenderPriority::Low,
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
        let overlay_width = 504.0f32;
        let overlay_height = 100.0f32;
        let overlay_x = width as f32 - overlay_width - 30.0;
        let overlay_y = height as f32 - overlay_height - 30.0;

        let hover_state = self.get_hover_state(self.mouse_pos.0, self.mouse_pos.1);
        let show_overlays = hover_state > 0 || self.page_input_active;

        if show_overlays {
            // 1. Draw bottom-right zoom overlay
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

                if let Some(border_r) = Rect::from_xywh(0.0, 0.0, ow as f32, oh as f32) {
                    ovl.fill_rect(border_r, &border_p, Transform::identity(), None);
                }
                if let Some(inner_r) = Rect::from_xywh(1.0, 1.0, ow as f32 - 2.0, oh as f32 - 2.0) {
                    ovl.fill_rect(inner_r, &bg_paint, Transform::identity(), None);
                }

                let btn_y_local = 12.0f32;
                let btn_size = 76.0f32;

                let draw_btn = |ovl: &mut Pixmap, x: f32, label: &str, hovered: bool| {
                    let mut p = Paint::default();
                    p.set_color_rgba8(if hovered { 70 } else { 40 }, if hovered { 70 } else { 40 }, if hovered { 70 } else { 40 }, 255);
                    if let Some(r) = Rect::from_xywh(x, btn_y_local, btn_size, btn_size) {
                        ovl.fill_rect(r, &p, Transform::identity(), None);
                    }
                    let mut tp = Paint::default(); tp.set_color_rgba8(255, 255, 255, 255); tp.anti_alias = true;
                    let tw = self.measure_text_width(label, 36.0, font);
                    self.draw_text(ovl, label, x + (btn_size - tw) / 2.0, btn_y_local + 52.0, 36.0, font, &tp);
                };

                draw_btn(&mut ovl, 20.0, "-", hover_state == 2);
                draw_btn(&mut ovl, 306.0, "+", hover_state == 3);
                draw_btn(&mut ovl, 408.0, "R", hover_state == 4);

                let current_fit_zoom = self.calculate_fit_zoom(width as u32, height as u32);
                let zoom_pct = format!("{:.0}%", (self.zoom / current_fit_zoom) * 100.0);
                let mut lp = Paint::default(); lp.set_color_rgba8(255, 255, 255, 255); lp.anti_alias = true;
                let lw = self.measure_text_width(&zoom_pct, 32.0, font);
                let lx = 96.0 + (210.0 - lw) / 2.0;
                self.draw_text(&mut ovl, &zoom_pct, lx, btn_y_local + 48.0, 32.0, font, &lp);

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
                            buffer[dst_idx] = ((s & 0xFF) << 16) | (s & 0xFF00) | ((s >> 16) & 0xFF);
                        } else {
                            let inv_a = 255 - a;
                            let bg_val = buffer[dst_idx];
                            let sr = s & 0xFF; let sg = (s >> 8) & 0xFF; let sb = (s >> 16) & 0xFF;
                            let br = (bg_val >> 16) & 0xFF; let bg_g = (bg_val >> 8) & 0xFF; let bb = bg_val & 0xFF;
                            let nr = (sr * a + br * inv_a) / 255; let ng = (sg * a + bg_g * inv_a) / 255; let nb = (sb * a + bb * inv_a) / 255;
                            buffer[dst_idx] = (nr << 16) | (ng << 8) | nb;
                        }
                    }
                }
            }

            // 2. Draw bottom-center pagination overlay
            let pag_overlay_width = 372.0f32;
            let pag_overlay_height = 100.0f32;
            let pag_overlay_x = (width as f32 - pag_overlay_width) / 2.0;
            let pag_overlay_y = height as f32 - pag_overlay_height - 30.0f32;

            let tow = (pag_overlay_width + 4.0) as u32;
            let toh = (pag_overlay_height + 4.0) as u32;
            let tox_px = (pag_overlay_x - 2.0).max(0.0) as usize;
            let toy_px = (pag_overlay_y - 2.0).max(0.0) as usize;

            if let Some(mut tovl) = Pixmap::new(tow, toh) {
                let font = &self.default_font.clone();
                let mut bg_paint = Paint::default();
                bg_paint.set_color_rgba8(25, 25, 25, 220);
                let mut border_p = Paint::default();
                border_p.set_color_rgba8(100, 100, 100, 255);

                if let Some(border_r) = Rect::from_xywh(0.0, 0.0, tow as f32, toh as f32) {
                    tovl.fill_rect(border_r, &border_p, Transform::identity(), None);
                }
                if let Some(inner_r) = Rect::from_xywh(1.0, 1.0, tow as f32 - 2.0, toh as f32 - 2.0) {
                    tovl.fill_rect(inner_r, &bg_paint, Transform::identity(), None);
                }

                let btn_y_local = 12.0f32;
                let btn_size = 76.0f32;

                let draw_nav_btn = |ovl: &mut Pixmap, x: f32, direction_left: bool, hovered: bool| {
                    let mut p = Paint::default();
                    p.set_color_rgba8(if hovered { 70 } else { 40 }, if hovered { 70 } else { 40 }, if hovered { 70 } else { 40 }, 255);
                    if let Some(r) = Rect::from_xywh(x, btn_y_local, btn_size, btn_size) {
                        ovl.fill_rect(r, &p, Transform::identity(), None);
                    }
                    
                    let mut pb = PathBuilder::new();
                    if direction_left {
                        pb.move_to(x + 42.5, btn_y_local + 24.0);
                        pb.line_to(x + 28.5, btn_y_local + 38.0);
                        pb.line_to(x + 42.5, btn_y_local + 52.0);
                        pb.line_to(x + 47.5, btn_y_local + 52.0);
                        pb.line_to(x + 33.5, btn_y_local + 38.0);
                        pb.line_to(x + 47.5, btn_y_local + 24.0);
                    } else {
                        pb.move_to(x + 33.5, btn_y_local + 24.0);
                        pb.line_to(x + 47.5, btn_y_local + 38.0);
                        pb.line_to(x + 33.5, btn_y_local + 52.0);
                        pb.line_to(x + 28.5, btn_y_local + 52.0);
                        pb.line_to(x + 42.5, btn_y_local + 38.0);
                        pb.line_to(x + 28.5, btn_y_local + 24.0);
                    }
                    pb.close();
                    
                    if let Some(path) = pb.finish() {
                        let mut tp = Paint::default();
                        tp.set_color_rgba8(255, 255, 255, 255);
                        tp.anti_alias = true;
                        ovl.fill_path(&path, &tp, FillRule::Winding, Transform::identity(), None);
                    }
                };

                // Previous page button `<`
                draw_nav_btn(&mut tovl, 12.0, true, hover_state == 21);

                // Next page button `>`
                draw_nav_btn(&mut tovl, 284.0, false, hover_state == 22);

                // Editable page input box background
                let mut input_bg = Paint::default();
                if self.page_input_active {
                    input_bg.set_color_rgba8(35, 35, 35, 255);
                } else if hover_state == 23 {
                    input_bg.set_color_rgba8(30, 30, 30, 255);
                } else {
                    input_bg.set_color_rgba8(15, 15, 15, 255);
                }
                
                let input_x = 100.0f32;
                let input_w = 80.0f32;

                let mut input_border = Paint::default();
                if self.page_input_active {
                    input_border.set_color_rgba8(100, 180, 255, 255); // blue highlight when active
                } else {
                    input_border.set_color_rgba8(100, 100, 100, 255);
                }

                if let Some(r) = Rect::from_xywh(input_x, btn_y_local, input_w, btn_size) {
                    tovl.fill_rect(r, &input_border, Transform::identity(), None);
                }
                if let Some(r) = Rect::from_xywh(input_x + 1.0, btn_y_local + 1.0, input_w - 2.0, btn_size - 2.0) {
                    tovl.fill_rect(r, &input_bg, Transform::identity(), None);
                }

                // Get page text to show
                let page_text = if self.page_input_active {
                    self.page_input_text.clone()
                } else {
                    (self.get_current_page_idx() + 1).to_string()
                };

                // Draw page text with optional cursor
                let mut text_to_draw = page_text;
                if self.page_input_active {
                    text_to_draw.push('|');
                }

                let mut lp = Paint::default(); lp.set_color_rgba8(255, 255, 255, 255); lp.anti_alias = true;
                let lw = self.measure_text_width(&text_to_draw, 32.0, font);
                let lx = input_x + (input_w - lw) / 2.0;
                self.draw_text(&mut tovl, &text_to_draw, lx, btn_y_local + 48.0, 32.0, font, &lp);

                // Draw "/ total" label
                let total_text = format!("/ {}", self.pages.len());
                let mut tp = Paint::default(); tp.set_color_rgba8(180, 180, 180, 255); tp.anti_alias = true;
                let total_x = 192.0f32;
                let total_w = 80.0f32;
                let tw = self.measure_text_width(&total_text, 32.0, font);
                let t_lx = total_x + (total_w - tw) / 2.0;
                self.draw_text(&mut tovl, &total_text, t_lx, btn_y_local + 48.0, 32.0, font, &tp);

                let tovl_data = tovl.data();
                let tovl_u32: &[u32] = unsafe {
                    std::slice::from_raw_parts(tovl_data.as_ptr() as *const u32, tovl_data.len() / 4)
                };
                for row in 0..toh as usize {
                    let dst_row = toy_px + row;
                    if dst_row >= height { break; }
                    for col in 0..tow as usize {
                        let dst_col = tox_px + col;
                        if dst_col >= width { break; }
                        let s = tovl_u32[row * tow as usize + col];
                        let a = (s >> 24) & 0xFF;
                        if a == 0 { continue; }
                        let dst_idx = dst_row * width + dst_col;
                        if a == 255 {
                            buffer[dst_idx] = ((s & 0xFF) << 16) | (s & 0xFF00) | ((s >> 16) & 0xFF);
                        } else {
                            let inv_a = 255 - a;
                            let bg_val = buffer[dst_idx];
                            let sr = s & 0xFF; let sg = (s >> 8) & 0xFF; let sb = (s >> 16) & 0xFF;
                            let br = (bg_val >> 16) & 0xFF; let bg_g = (bg_val >> 8) & 0xFF; let bb = bg_val & 0xFF;
                            let nr = (sr * a + br * inv_a) / 255; let ng = (sg * a + bg_g * inv_a) / 255; let nb = (sb * a + bb * inv_a) / 255;
                            buffer[dst_idx] = (nr << 16) | (ng << 8) | nb;
                        }
                    }
                }
            }
        }

        // --- Left Menu Overlay ---
        let _menu_btn_x = 30.0f32;
        let menu_btn_y = height as f32 - 100.0 - 30.0;
        let menu_btn_w = 84.0f32;
        let menu_btn_h = 100.0f32;
        
        let menu_w = 364.0f32;
        let menu_h = 448.0f32;
        let menu_x = 30.0f32;
        let menu_y = height as f32 - 100.0 - 30.0 - menu_h - 10.0;

        let left_ow = menu_w as u32 + 4;
        let left_oh = (menu_btn_y + menu_btn_h - menu_y) as u32 + 4;

        if hover_state >= 9 || self.left_menu_open {
            if let Some(mut lovl) = Pixmap::new(left_ow, left_oh) {
                let font = &self.default_font.clone();
                let mut bg_paint = Paint::default();
                bg_paint.set_color_rgba8(25, 25, 25, 220);
                let mut border_p = Paint::default();
                border_p.set_color_rgba8(100, 100, 100, 255);

                let btn_rect_y = menu_btn_y - menu_y;
                if let Some(r) = Rect::from_xywh(0.0, btn_rect_y, menu_btn_w, menu_btn_h) {
                    lovl.fill_rect(r, &border_p, Transform::identity(), None);
                }
                if let Some(r) = Rect::from_xywh(1.0, btn_rect_y + 1.0, menu_btn_w - 2.0, menu_btn_h - 2.0) {
                    let mut p = bg_paint.clone();
                    if hover_state == 9 { p.set_color_rgba8(70, 70, 70, 255); }
                    lovl.fill_rect(r, &p, Transform::identity(), None);
                }
                
                let mut icon_p = Paint::default(); icon_p.set_color_rgba8(255, 255, 255, 255);
                for i in 0..3 {
                    if let Some(r) = Rect::from_xywh(21.0, btn_rect_y + 34.0 + i as f32 * 12.0, 42.0, 6.0) {
                        lovl.fill_rect(r, &icon_p, Transform::identity(), None);
                    }
                }

                if self.left_menu_open {
                    if let Some(r) = Rect::from_xywh(0.0, 0.0, menu_w, menu_h) {
                        lovl.fill_rect(r, &border_p, Transform::identity(), None);
                    }
                    if let Some(r) = Rect::from_xywh(1.0, 1.0, menu_w - 2.0, menu_h - 2.0) {
                        lovl.fill_rect(r, &bg_paint, Transform::identity(), None);
                    }

                    let items = [
                        "Single-page view",
                        "Two-page view",
                        "Enable scrolling",
                        "Actual size",
                        "Zoom to page level",
                        "Fit to width",
                        "Fit height",
                        "Fit visible content"
                    ];

                    let item_h = menu_h / 8.0;
                    let mut text_p = Paint::default(); text_p.set_color_rgba8(255, 255, 255, 255); text_p.anti_alias = true;
                    
                    for (i, text) in items.iter().enumerate() {
                        let y = i as f32 * item_h;
                        if hover_state == 10 + i as u8 {
                            let mut hp = Paint::default(); hp.set_color_rgba8(70, 70, 70, 255);
                            if let Some(r) = Rect::from_xywh(1.0, y + 1.0, menu_w - 2.0, item_h - 2.0) {
                                lovl.fill_rect(r, &hp, Transform::identity(), None);
                            }
                        }
                        
                        // Highlight selected layout mode
                        if (i == 0 && self.layout_mode == LayoutMode::SinglePage) ||
                           (i == 1 && self.layout_mode == LayoutMode::TwoPage) ||
                           (i == 2 && self.layout_mode == LayoutMode::Continuous) {
                            let mut ind_p = Paint::default(); ind_p.set_color_rgba8(100, 200, 255, 255);
                            if let Some(r) = Rect::from_xywh(8.0, y + item_h / 2.0 - 6.0, 12.0, 12.0) {
                                lovl.fill_rect(r, &ind_p, Transform::identity(), None);
                            }
                        }
                        
                        self.draw_text(&mut lovl, text, 28.0, y + 38.0, 28.0, font, &text_p);
                    }
                }

                let ox_px = menu_x as usize;
                let oy_px = menu_y as usize;
                let lovl_data = lovl.data();
                let lovl_u32: &[u32] = unsafe {
                    std::slice::from_raw_parts(lovl_data.as_ptr() as *const u32, lovl_data.len() / 4)
                };
                for row in 0..left_oh as usize {
                    let dst_row = oy_px + row;
                    if dst_row >= height { break; }
                    for col in 0..left_ow as usize {
                        let dst_col = ox_px + col;
                        if dst_col >= width { break; }
                        let s = lovl_u32[row * left_ow as usize + col];
                        let a = (s >> 24) & 0xFF;
                        if a == 0 { continue; }
                        let dst_idx = dst_row * width + dst_col;
                        if a == 255 {
                            buffer[dst_idx] = ((s & 0xFF) << 16) | (s & 0xFF00) | ((s >> 16) & 0xFF);
                        } else {
                            let inv_a = 255 - a;
                            let bg_val = buffer[dst_idx];
                            let sr = s & 0xFF; let sg = (s >> 8) & 0xFF; let sb = (s >> 16) & 0xFF;
                            let br = (bg_val >> 16) & 0xFF; let bg_g = (bg_val >> 8) & 0xFF; let bb = bg_val & 0xFF;
                            let nr = (sr * a + br * inv_a) / 255; let ng = (sg * a + bg_g * inv_a) / 255; let nb = (sb * a + bb * inv_a) / 255;
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
                    .with_title("Adobo Reader")
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
                self.rendered_zoom = self.zoom;
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
                    self.rendered_zoom = self.zoom;
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
                
                if self.page_input_active && hover != 23 {
                    self.page_input_active = false;
                    if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                }

                if hover == 9 {
                    self.left_menu_open = !self.left_menu_open;
                    if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                } else if hover >= 10 && hover <= 17 {
                    self.left_menu_open = false;
                    let idx = hover - 10;
                    match idx {
                        0 => { self.layout_mode = LayoutMode::SinglePage; self.recalculate_layout(); }
                        1 => { self.layout_mode = LayoutMode::TwoPage; self.recalculate_layout(); }
                        2 => { self.layout_mode = LayoutMode::Continuous; self.recalculate_layout(); }
                        3 => { self.set_target_zoom(1.0); } // Actual size
                        4 => { let z = self.calculate_fit_height_zoom(self.window_size.height); self.set_target_zoom(z); } // Zoom to page level
                        5 => { let z = self.calculate_fit_width_zoom(self.window_size.width); self.set_target_zoom(z); } // Fit width
                        6 => { let z = self.calculate_fit_height_zoom(self.window_size.height); self.set_target_zoom(z); } // Fit height
                        7 => { let z = self.calculate_fit_width_zoom(self.window_size.width) * 1.1; self.set_target_zoom(z); } // Fit visible content
                        _ => {}
                    }
                } else if hover > 1 && hover < 9 {
                    self.left_menu_open = false;
                    let old_zoom = self.zoom;
                    let new_zoom = match hover {
                        2 => (old_zoom / 1.1).clamp(0.1, 10.0),
                        3 => (old_zoom * 1.1).clamp(0.1, 10.0),
                        4 => self.calculate_fit_zoom(self.window_size.width, self.window_size.height),
                        _ => old_zoom,
                    };
                    if (new_zoom - old_zoom).abs() > 0.0001 {
                        self.set_target_zoom(new_zoom);
                    }
                } else if hover == 21 {
                    self.left_menu_open = false;
                    let current_idx = self.get_current_page_idx();
                    if current_idx > 0 {
                        self.jump_to_page(current_idx - 1);
                    }
                } else if hover == 22 {
                    self.left_menu_open = false;
                    let current_idx = self.get_current_page_idx();
                    if current_idx + 1 < self.pages.len() {
                        self.jump_to_page(current_idx + 1);
                    }
                } else if hover == 23 {
                    self.left_menu_open = false;
                    self.page_input_active = true;
                    self.page_input_text = (self.get_current_page_idx() + 1).to_string();
                    if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                } else {
                    if self.left_menu_open {
                        self.left_menu_open = false;
                        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
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
                            self.last_zoom_change_time = std::time::Instant::now();
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
                                  self.last_zoom_change_time = std::time::Instant::now();
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
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == winit::event::ElementState::Pressed {
                    if self.page_input_active {
                        match &event.logical_key {
                            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Backspace) => {
                                self.page_input_text.pop();
                                if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                                return;
                            }
                            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                                if let Ok(page_num) = self.page_input_text.trim().parse::<usize>() {
                                    if page_num > 0 && page_num <= self.pages.len() {
                                        self.jump_to_page(page_num - 1);
                                    }
                                }
                                self.page_input_active = false;
                                if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                                return;
                            }
                            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                                self.page_input_active = false;
                                if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                                return;
                            }
                            winit::keyboard::Key::Character(c_str) => {
                                if c_str.len() == 1 && c_str.chars().next().unwrap().is_ascii_digit() {
                                    if self.page_input_text.len() < 5 {
                                        self.page_input_text.push_str(c_str);
                                    }
                                }
                                if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                                return;
                            }
                            _ => {}
                        }
                    } else {
                        if let winit::keyboard::PhysicalKey::Code(code) = event.physical_key {
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
                                    self.last_zoom_change_time = std::time::Instant::now();
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
                                _ => {}
                            }
                            if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                        }
                    }
                }
            }
            _ => (),
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Debounce of 200ms for zoom rendering
        if (self.zoom - self.rendered_zoom).abs() > 0.0001 
            && self.last_zoom_change_time.elapsed() >= std::time::Duration::from_millis(200) 
        {
            self.rendered_zoom = self.zoom;
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }

        let mut got_any = false;
        let epoch = self.render_epoch.load(Ordering::SeqCst);
        while let Ok(msg) = self.rx_worker.try_recv() {
            match msg {
                WorkerMessage::PageRendered { page_idx, zoom, pixmap } => {
                    if (zoom - self.zoom).abs() < 0.001 {
                        self.page_images.borrow_mut().insert(page_idx, CachedPage { pixmap, zoom });
                        self.record_access_and_evict(page_idx);
                        let zoom_key = (self.zoom * 1000.0) as u32;
                        self.requested_pages.borrow_mut().remove(&(page_idx, zoom_key));
                        got_any = true;
                    }
                }
                WorkerMessage::PageRenderAborted { page_idx, zoom } => {
                    let zoom_key = (zoom * 1000.0) as u32;
                    self.requested_pages.borrow_mut().remove(&(page_idx, zoom_key));
                    got_any = true;
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
        let epoch_clone = Arc::new(AtomicUsize::new(0));
        let epoch_clone2 = epoch_clone.clone();
        
        std::thread::spawn(move || {
            run_worker_thread(
                epoch_clone,
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
            render_epoch: epoch_clone2,
            scroll_x: 0.0,
            scroll_y: 0.0,
            target_scroll_x: 0.0,
            target_scroll_y: 0.0,
            last_scroll_y: 0.0,
            scroll_down_direction: true,
            zoom: 1.0,
            rendered_zoom: 1.0,
            last_zoom_change_time: std::time::Instant::now(),
            modifiers: winit::keyboard::ModifiersState::default(),
            mouse_pos: (0.0, 0.0),
            window_size: winit::dpi::PhysicalSize::new(0, 0),
            zoom_initialized: false,
            default_font,
            logo_pixmap: Pixmap::load_png("assets/logo.png").ok(),
            layout_mode: LayoutMode::Continuous,
            left_menu_open: false,
            page_input_active: false,
            page_input_text: String::new(),
        };
        event_loop.run_app(&mut app)?;
        Ok(())
    }
}

fn run_worker_thread(
    render_epoch: Arc<AtomicUsize>,
    pdf_path: String,
    fonts: std::collections::HashMap<String, Arc<FontVec>>,
    default_font: Arc<FontVec>,
    rx_request: std::sync::mpsc::Receiver<RenderRequest>,
    tx_response: std::sync::mpsc::Sender<WorkerMessage>,
    proxy: winit::event_loop::EventLoopProxy<()>,
) {
    if pdf_path.is_empty() {
        return;
    }
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
    let page_commands_cache = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let page_paths_cache: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<(usize, &'static str), Option<Path>>>> = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

    let font_keys = [
        "serif_regular", "serif_bold", "serif_italic", "serif_bold_italic",
        "sans_regular", "sans_bold", "sans_italic", "mono_regular"
    ];

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
            let prio_cmp = b.priority.cmp(&a.priority);
            if prio_cmp != std::cmp::Ordering::Equal {
                return prio_cmp;
            }
            let dist_a = get_distance_to_viewport(a.page_y, a.page_height, a.window_height);
            let dist_b = get_distance_to_viewport(b.page_y, b.page_height, b.window_height);
            dist_a.partial_cmp(&dist_b).unwrap_or(std::cmp::Ordering::Equal)
        });

        if local_queue.is_empty() {
            continue;
        }

        let requests_to_process = std::mem::take(&mut local_queue);
        for request in requests_to_process {
            let page_idx = request.page_idx;
            let zoom = request.zoom;
            let current_epoch = render_epoch.load(Ordering::Relaxed);
            if request.epoch < current_epoch {
                continue;
            }

            let page_commands_cache = page_commands_cache.clone();
            let page_paths_cache = page_paths_cache.clone();
            let tx_response = tx_response.clone();
            let proxy = proxy.clone();
            let render_epoch = render_epoch.clone();
            let mut parser = parser.clone();
            let interpreter = interpreter.clone();
            let font_keys = font_keys.clone();
            let fonts = fonts.clone();
            let default_font = default_font.clone();

            rayon::spawn(move || {
                let select_font_and_key = |basefont: &str| -> (&Arc<FontVec>, &'static str) {
                    let key = map_font_name(basefont);
                    if fonts.contains_key(key) {
                        (&fonts[key], key)
                    } else {
                        (&default_font, "default")
                    }
                };

                let start_time = std::time::Instant::now();
                let render_completed = 'render: {
                let commands = {
                    let cache = page_commands_cache.lock().unwrap();
                    cache.get(&page_idx).cloned()
                };
                let commands = if let Some(cmds) = commands {
            cmds
        } else {
            if render_epoch.load(Ordering::Relaxed) != current_epoch {
                break 'render false;
            }
            let page_rect = parser.get_page_rect(page_idx).unwrap_or(crate::parser::PageRect {
                x: 0.0, y: 0.0, width: 595.0, height: 842.0
            });
            let parsed_cmds = match parser.get_page_content(page_idx) {
                Ok(content) => {
                    let page_images = parser.get_page_images(page_idx).unwrap_or_default();
                    if render_epoch.load(Ordering::Relaxed) != current_epoch {
                        break 'render false;
                    }
                    if let Some(cmds) = interpreter.process(page_idx, &content, page_rect, Some(&render_epoch), Some(current_epoch), &page_images) {
                        cmds
                    } else {
                        break 'render false;
                    }
                }
                Err(_) => Vec::new(),
            };
            if render_epoch.load(Ordering::Relaxed) != current_epoch {
                break 'render false;
            }
            let mut cache = page_commands_cache.lock().unwrap();
            cache.insert(page_idx, parsed_cmds.clone());
            parsed_cmds
        };

        let page_rect = parser.get_page_rect(page_idx).unwrap_or(crate::parser::PageRect {
            x: 0.0, y: 0.0, width: 595.0, height: 842.0
        });

        // 2. Build font paths for this page if not cached
        for &font_key in &font_keys {
            let has_path = {
                let cache = page_paths_cache.lock().unwrap();
                cache.contains_key(&(page_idx, font_key))
            };
            if !has_path {
                let mut path_builder = PathBuilder::new();
                let mut has_glyphs = false;

                for cmd in commands.iter() {
                    if render_epoch.load(Ordering::Relaxed) != current_epoch {
                        break 'render false;
                    }
                    if let DrawCommand::Text { chars, local_y, size, font_name, .. } = cmd {
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
                }

                let path_opt = if has_glyphs {
                    path_builder.finish()
                } else {
                    None
                };
                let mut cache = page_paths_cache.lock().unwrap();
                cache.insert((page_idx, font_key), path_opt);
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
                    let path_opt = {
                        let cache = page_paths_cache.lock().unwrap();
                        cache.get(&(page_idx, font_key)).cloned().flatten()
                    };
                    if let Some(p) = path_opt {
                        let transform = Transform::from_scale(zoom, zoom);
                        page_pixmap.fill_path(&p, &text_paint, FillRule::Winding, transform, None);
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
                println!("Page {} rendered in {} ms", page_idx, start_time.elapsed().as_millis());
            }
        }
        true
        }; // end 'render

        if !render_completed {
            {
                let mut cache = page_paths_cache.lock().unwrap();
                for &font_key in &font_keys {
                    cache.remove(&(page_idx, font_key));
                }
            }
            tx_response.send(WorkerMessage::PageRenderAborted { page_idx, zoom }).ok();
            proxy.send_event(()).ok();
        }
        }); // end rayon::spawn
        } // end for request in requests_to_process
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

