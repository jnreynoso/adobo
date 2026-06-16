use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};
use vello::{kurbo, Scene};
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
        scene: Scene,
    },
    PageRenderAborted {
        page_idx: usize,
        zoom: f32,
    },
}

pub struct CachedPage {
    pub scene: Scene,
    pub zoom: f32,
}

pub struct App {
    window: Option<Arc<Window>>,
    render_cx: vello::util::RenderContext,
    render_surface: Option<vello::util::RenderSurface<'static>>,
    renderer: Option<vello::Renderer>,
    pdf_path: String,
    pages: Vec<PageInfo>,
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
    logo_image: Option<vello::peniko::ImageData>,
    logo_rgba: Option<Vec<u8>>,
    logo_width: u32,
    logo_height: u32,
    window_icon: Option<winit::window::Icon>,
    layout_mode: LayoutMode,
    left_menu_open: bool,
    top_menu_open: bool,
    recent_menu_open: bool,
    page_input_active: bool,
    page_input_text: String,
    lecturas_page_idx: usize,
    lecturas_search_query: String,
    lecturas_search_active: bool,
    surface_size: (u32, u32),
    last_draw_time: std::time::Instant,
    loading_timeout: Option<std::time::Instant>,
    loading_lectura_idx: Option<usize>,
    kill_pid: Option<u32>,
    pdf_title: String,
    pdf_author: String,
    view_lecturas: bool,
    is_dragging_scrollbar: bool,
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

    #[allow(dead_code)]
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

    fn measure_text_width(&self, text: &str, size: f32, font: &FontVec) -> f32 {
        let scale_factor = size / font.units_per_em().unwrap_or(1000.0);
        let mut width = 0.0;
        for c in text.chars() {
            let glyph_id = font.glyph_id(c);
            width += font.h_advance_unscaled(glyph_id) * scale_factor;
        }
        width
    }

    fn truncate_text_with_ellipsis(&self, text: &str, max_width: f64, size: f32, font: &FontVec) -> String {
        if self.measure_text_width(text, size, font) as f64 <= max_width {
            return text.to_string();
        }
        
        let mut truncated = text.to_string();
        while !truncated.is_empty() {
            truncated.pop();
            let candidate = format!("{}...", truncated);
            if self.measure_text_width(&candidate, size, font) as f64 <= max_width {
                return candidate;
            }
        }
        "...".to_string()
    }

    fn draw_text_to_scene(&self, scene: &mut Scene, text: &str, start_x: f64, y: f64, size: f64, font: &FontVec, color: vello::peniko::Color) {
        let scale_factor = size / font.units_per_em().unwrap_or(1000.0) as f64;
        let mut current_x = start_x;
        for c in text.chars() {
            let glyph_id = font.glyph_id(c);
            let actual_w = font.h_advance_unscaled(glyph_id) as f64 * scale_factor;
            
            if let Some(outline) = font.outline(glyph_id) {
                let mut path = kurbo::BezPath::new();
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
                        path.move_to((current_x + start_p.x as f64 * scale_factor, y - start_p.y as f64 * scale_factor));
                    }
                    match curve {
                        ab_glyph::OutlineCurve::Line(_, p2) => {
                            path.line_to((current_x + p2.x as f64 * scale_factor, y - p2.y as f64 * scale_factor));
                            last_point = Some(p2);
                        }
                        ab_glyph::OutlineCurve::Quad(_, p2, p3) => {
                            path.quad_to(
                                (current_x + p2.x as f64 * scale_factor, y - p2.y as f64 * scale_factor),
                                (current_x + p3.x as f64 * scale_factor, y - p3.y as f64 * scale_factor)
                            );
                            last_point = Some(p3);
                        }
                        ab_glyph::OutlineCurve::Cubic(_, p2, p3, p4) => {
                            path.curve_to(
                                (current_x + p2.x as f64 * scale_factor, y - p2.y as f64 * scale_factor),
                                (current_x + p3.x as f64 * scale_factor, y - p3.y as f64 * scale_factor),
                                (current_x + p4.x as f64 * scale_factor, y - p4.y as f64 * scale_factor)
                            );
                            last_point = Some(p4);
                        }
                    }
                }
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    kurbo::Affine::IDENTITY,
                    color,
                    None,
                    &path,
                );
            }
            current_x += actual_w;
        }
    }

    fn draw_splash_screen_to_scene(&self, scene: &mut Scene, width: f64, height: f64) {
        let font = &self.default_font;
        let title = "Adobo";
        let title_size = 48.0;
        
        let splash_w = 400.0;
        let splash_h = 400.0;
        let ox = (width - splash_w) / 2.0;
        let oy = (height - splash_h) / 2.0;

        // Draw background of splash
        let bg_rect = kurbo::Rect::new(ox, oy, ox + splash_w, oy + splash_h);
        scene.fill(
            vello::peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            vello::peniko::Color::from_rgb8(18, 18, 18),
            None,
            &bg_rect,
        );

        // Draw logo
        if let Some(ref logo) = self.logo_image {
            let lw = 160.0;
            let lh = 160.0;
            let lx = ox + (splash_w - lw) / 2.0;
            let ly = oy + 40.0;
            
            let sx = lw / logo.width as f64;
            let sy = lh / logo.height as f64;
            let transform = kurbo::Affine::translate((lx, ly)) * kurbo::Affine::scale_non_uniform(sx, sy);
            
            let image_brush = vello::peniko::Brush::Image(vello::peniko::ImageBrush::new(logo.clone()));
            let fill_rect = kurbo::Rect::new(0.0, 0.0, logo.width as f64, logo.height as f64);
            scene.fill(
                vello::peniko::Fill::NonZero,
                transform,
                &image_brush,
                None,
                &fill_rect,
            );
        }

        // Draw title
        let text_y = oy + 260.0;
        let tw = self.measure_text_width(title, title_size as f32, font) as f64;
        let tx = ox + (splash_w - tw) / 2.0;
        self.draw_text_to_scene(scene, title, tx, text_y, title_size, font, vello::peniko::Color::WHITE);

        // Draw subtitle
        let sub = "Cargando documento...";
        let sub_size = 16.0;
        let sw = self.measure_text_width(sub, sub_size as f32, font) as f64;
        let sx = ox + (splash_w - sw) / 2.0;
        self.draw_text_to_scene(scene, sub, sx, text_y + 40.0, sub_size, font, vello::peniko::Color::from_rgb8(150, 150, 150));
    }

    fn draw_welcome_screen_to_scene(&self, scene: &mut Scene, width: f64, height: f64) {
        let font = &self.default_font;
        let bg_color = vello::peniko::Color::from_rgb8(15, 18, 25);
        let bg_rect = kurbo::Rect::new(0.0, 0.0, width, height);
        scene.fill(
            vello::peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            bg_color,
            None,
            &bg_rect,
        );

        let title = "Bienvenido a Adobo";
        let title_size = 72.0;
        let tw = self.measure_text_width(title, title_size as f32, font) as f64;
        let tx = (width - tw) / 2.0;

        let paragraph1 = "Adobo es un proyecto libre de visualización de PDF escrito en Rust.";
        let paragraph2 = "Se enfoca en brindarle al usuario una lectura placentera y minimalista,";
        let paragraph3 = "eliminando las funciones pesadas que los lectores tradicionales no necesitan.";

        let sub_size = 32.0;
        let text_color = vello::peniko::Color::from_rgb8(210, 210, 210);

        let w1 = self.measure_text_width(paragraph1, sub_size as f32, font) as f64;
        let w2 = self.measure_text_width(paragraph2, sub_size as f32, font) as f64;
        let w3 = self.measure_text_width(paragraph3, sub_size as f32, font) as f64;

        let lw = 384.0;
        let lh = 384.0;
        let total_content_height = lh + 80.0 + 80.0 + 80.0 + 80.0;
        let start_y = ((height - total_content_height) / 2.0 - 80.0).max(50.0);

        let ly = start_y;
        let ty = start_y + lh + 80.0;
        let ty_p1 = ty + 100.0;
        let ty_p2 = ty_p1 + 50.0;
        let ty_p3 = ty_p2 + 50.0;

        self.draw_text_to_scene(scene, title, tx, ty, title_size, font, vello::peniko::Color::WHITE);

        self.draw_text_to_scene(scene, paragraph1, (width - w1) / 2.0, ty_p1, sub_size, font, text_color);
        self.draw_text_to_scene(scene, paragraph2, (width - w2) / 2.0, ty_p2, sub_size, font, text_color);
        self.draw_text_to_scene(scene, paragraph3, (width - w3) / 2.0, ty_p3, sub_size, font, text_color);

        // Draw logo
        if let Some(ref rgba) = self.logo_rgba {
            let logo = vello::peniko::ImageData {
                data: vello::peniko::Blob::new(std::sync::Arc::new(rgba.clone())),
                format: vello::peniko::ImageFormat::Rgba8,
                alpha_type: vello::peniko::ImageAlphaType::Alpha,
                width: self.logo_width,
                height: self.logo_height,
            };
            let lx = (width - lw) / 2.0;
            
            let sx = lw / logo.width as f64;
            let sy = lh / logo.height as f64;
            let transform = kurbo::Affine::translate((lx, ly)) * kurbo::Affine::scale_non_uniform(sx, sy);
            
            let image_brush = vello::peniko::Brush::Image(vello::peniko::ImageBrush::new(logo));
            let fill_rect = kurbo::Rect::new(0.0, 0.0, self.logo_width as f64, self.logo_height as f64);
            scene.fill(
                vello::peniko::Fill::NonZero,
                transform,
                &image_brush,
                None,
                &fill_rect,
            );
        }
    }

    fn draw_lecturas_to_scene(&self, scene: &mut Scene, width: f64, height: f64, hover_state: u8) {
        let font = &self.default_font;
        let bg_color = vello::peniko::Color::from_rgb8(15, 18, 25);
        let bg_rect = kurbo::Rect::new(0.0, 0.0, width, height);
        scene.fill(
            vello::peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            bg_color,
            None,
            &bg_rect,
        );

        let title = "Lecturas Actuales";
        let title_size = 72.0;
        let tw = self.measure_text_width(title, title_size as f32, font) as f64;
        self.draw_text_to_scene(scene, title, (width - tw) / 2.0, 100.0, title_size, font, vello::peniko::Color::WHITE);

        // Volver button
        let stroke_style = kurbo::Stroke::new(2.0);
        let border_color = vello::peniko::Color::from_rgb8(100, 100, 100);
        let volver_rect = kurbo::RoundedRect::new(30.0, 30.0, 190.0, 94.0, 8.0);
        let volver_bg = vello::peniko::Color::from_rgb8(
            if hover_state == 50 { 70 } else { 25 },
            if hover_state == 50 { 70 } else { 25 },
            if hover_state == 50 { 70 } else { 25 },
        );
        scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, volver_bg, None, &volver_rect);
        scene.stroke(&stroke_style, kurbo::Affine::IDENTITY, border_color, None, &volver_rect);

        self.draw_text_to_scene(scene, "Volver", 30.0 + 24.0, 30.0 + 44.0, 36.0, font, vello::peniko::Color::WHITE);

        let query = self.lecturas_search_query.to_lowercase();
        let readings: Vec<_> = crate::db::get_all_readings().into_iter().filter(|(p, info)| {
            if query.is_empty() { return true; }
            let name = std::path::Path::new(p).file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
            let author = info.author.to_lowercase();
            name.contains(&query) || author.contains(&query)
        }).collect();

        // Search bar
        let search_w = 600.0;
        let search_h = 60.0;
        let search_x = (width - search_w) / 2.0;
        let search_y = 160.0;
        let search_rect = kurbo::RoundedRect::new(search_x, search_y, search_x + search_w, search_y + search_h, 20.0);
        let search_bg = if self.lecturas_search_active { vello::peniko::Color::from_rgb8(50, 55, 70) } else { vello::peniko::Color::from_rgb8(30, 34, 45) };
        scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, search_bg, None, &search_rect);
        scene.stroke(&stroke_style, kurbo::Affine::IDENTITY, border_color, None, &search_rect);
        
        let display_text = if self.lecturas_search_query.is_empty() && !self.lecturas_search_active {
            "Buscar por título o autor..."
        } else {
            &self.lecturas_search_query
        };
        let t_color = if self.lecturas_search_query.is_empty() && !self.lecturas_search_active { vello::peniko::Color::from_rgb8(150, 150, 150) } else { vello::peniko::Color::WHITE };
        
        // Add a blinking cursor if active
        let mut final_text = display_text.to_string();
        if self.lecturas_search_active && (std::time::Instant::now().elapsed().as_millis() % 1000) < 500 {
            final_text.push('|');
        }
        self.draw_text_to_scene(scene, &final_text, search_x + 30.0, search_y + 40.0, 28.0, font, t_color);

        if readings.is_empty() {
            let msg = if query.is_empty() { "No hay lecturas en progreso." } else { "No se encontraron lecturas." };
            let mw = self.measure_text_width(msg, 32.0, font) as f64;
            self.draw_text_to_scene(scene, msg, (width - mw) / 2.0, height / 2.0, 32.0, font, vello::peniko::Color::from_rgb8(150, 155, 170));
            return;
        }

        let start_y = 260.0;
        let item_h = 140.0;
        let gap = 20.0;
        let list_w = (width - 100.0).max(800.0);
        let list_x = (width - list_w) / 2.0;
        
        let items_per_page = ((height - 400.0) / (item_h + gap)).floor().max(1.0) as usize;
        let total_pages = (readings.len() + items_per_page - 1) / items_per_page;
        let page_idx = self.lecturas_page_idx.min(total_pages.saturating_sub(1));
        
        let start_idx = page_idx * items_per_page;
        let end_idx = (start_idx + items_per_page).min(readings.len());
        let current_page_items = &readings[start_idx..end_idx];

        for (idx, (path, progress)) in current_page_items.iter().enumerate() {
            let item_y = start_y + (idx as f64 * (item_h + gap));
            let item_rect = kurbo::RoundedRect::new(list_x, item_y, list_x + list_w, item_y + item_h, 12.0);
            
            // Item Background
            scene.fill(
                vello::peniko::Fill::NonZero,
                kurbo::Affine::IDENTITY,
                vello::peniko::Color::from_rgb8(30, 34, 45),
                None,
                &item_rect,
            );
            // Item Border
            scene.stroke(
                &kurbo::Stroke::new(1.0),
                kurbo::Affine::IDENTITY,
                vello::peniko::Color::from_rgb8(50, 55, 70),
                None,
                &item_rect,
            );

            // Title (from filename) and Author
            let display_name = std::path::Path::new(path).file_stem().unwrap_or_default().to_string_lossy();
            self.draw_text_to_scene(scene, &display_name, list_x + 30.0, item_y + 55.0, 36.0, font, vello::peniko::Color::WHITE);
            let author_txt = format!("Por: {}", progress.author);
            self.draw_text_to_scene(scene, &author_txt, list_x + 30.0, item_y + 100.0, 24.0, font, vello::peniko::Color::from_rgb8(170, 175, 190));

            // Progress text
            let prog_pct = progress.percentage();
            let prog_txt = format!("Pág {} de {} ({:.1}%)", progress.current_page + 1, progress.total_pages, prog_pct);
            let prog_txt_w = self.measure_text_width(&prog_txt, 24.0, font) as f64;
            self.draw_text_to_scene(scene, &prog_txt, list_x + list_w - 220.0 - prog_txt_w - 30.0, item_y + 65.0, 24.0, font, vello::peniko::Color::from_rgb8(180, 185, 200));

            // Progress bar
            let bar_w = 300.0;
            let bar_x = list_x + list_w - 220.0 - bar_w - 30.0;
            let bar_y = item_y + 85.0;
            let bar_bg = kurbo::RoundedRect::new(bar_x, bar_y, bar_x + bar_w, bar_y + 8.0, 4.0);
            scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, vello::peniko::Color::from_rgb8(45, 50, 65), None, &bar_bg);
            
            let fill_w = bar_w * (prog_pct as f64 / 100.0);
            if fill_w > 0.0 {
                let bar_fg = kurbo::RoundedRect::new(bar_x, bar_y, bar_x + fill_w, bar_y + 8.0, 4.0);
                scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, vello::peniko::Color::from_rgb8(80, 200, 150), None, &bar_fg);
            }

            // Leer button
            let btn_w = 180.0;
            let btn_h = 60.0;
            let btn_x = list_x + list_w - btn_w - 30.0;
            let btn_y = item_y + 40.0;
            let is_hover = hover_state == (100 + idx) as u8;
            let btn_bg = if is_hover { vello::peniko::Color::from_rgb8(80, 120, 220) } else { vello::peniko::Color::from_rgb8(50, 90, 190) };
            
            let btn_rect = kurbo::RoundedRect::new(btn_x, btn_y, btn_x + btn_w, btn_y + btn_h, 8.0);
            scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, btn_bg, None, &btn_rect);
            
            if self.loading_lectura_idx == Some(start_idx + idx) && self.loading_timeout.is_some() {
                let ms = std::time::Instant::now().elapsed().as_millis();
                let dots = match (ms / 300) % 4 {
                    0 => "",
                    1 => ".",
                    2 => "..",
                    _ => "...",
                };
                let txt = format!("Cargando{}", dots);
                let tw = self.measure_text_width(&txt, 20.0, font) as f64;
                self.draw_text_to_scene(scene, &txt, btn_x + (btn_w - tw) / 2.0, btn_y + 36.0, 20.0, font, vello::peniko::Color::WHITE);
            } else {
                let leer_txt = "Leer";
                let leer_tw = self.measure_text_width(leer_txt, 28.0, font) as f64;
                self.draw_text_to_scene(scene, leer_txt, btn_x + (btn_w - leer_tw) / 2.0, btn_y + 40.0, 28.0, font, vello::peniko::Color::WHITE);
            }
        }

        if total_pages > 1 {
            let pag_y = height - 80.0;
            
            // Prev button
            if page_idx > 0 {
                let prev_rect = kurbo::RoundedRect::new(width / 2.0 - 150.0, pag_y, width / 2.0 - 50.0, pag_y + 50.0, 8.0);
                let prev_bg = if hover_state == 51 { vello::peniko::Color::from_rgb8(80, 120, 220) } else { vello::peniko::Color::from_rgb8(50, 90, 190) };
                scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, prev_bg, None, &prev_rect);
                self.draw_text_to_scene(scene, "Anterior", width / 2.0 - 130.0, pag_y + 34.0, 24.0, font, vello::peniko::Color::WHITE);
            }
            
            let pag_text = format!("Página {} de {}", page_idx + 1, total_pages);
            let pag_tw = self.measure_text_width(&pag_text, 24.0, font) as f64;
            self.draw_text_to_scene(scene, &pag_text, (width - pag_tw) / 2.0, pag_y + 34.0, 24.0, font, vello::peniko::Color::WHITE);
            
            // Next button
            if page_idx < total_pages - 1 {
                let next_rect = kurbo::RoundedRect::new(width / 2.0 + 50.0, pag_y, width / 2.0 + 150.0, pag_y + 50.0, 8.0);
                let next_bg = if hover_state == 52 { vello::peniko::Color::from_rgb8(80, 120, 220) } else { vello::peniko::Color::from_rgb8(50, 90, 190) };
                scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, next_bg, None, &next_rect);
                self.draw_text_to_scene(scene, "Siguiente", width / 2.0 + 65.0, pag_y + 34.0, 24.0, font, vello::peniko::Color::WHITE);
            }
        }
    }


    fn draw_ui_overlays_to_scene(&self, scene: &mut Scene, width: f64, height: f64, hover_state: u8) {
        let font = &self.default_font;
        let has_pdf = !self.pages.is_empty();

        let stroke_style = kurbo::Stroke::new(2.0);
        let border_color = vello::peniko::Color::from_rgb8(100, 100, 100);
        let bg_color = vello::peniko::Color::from_rgba8(25, 25, 25, 220);

        // TOP-LEFT MENU (Abrir Archivo) - only visible when no PDF is loaded
        if !has_pdf {
            let top_menu_btn_x = 30.0;
            let top_menu_btn_y = 30.0;
            let top_menu_btn_w = 160.0;
            let top_menu_btn_h = 64.0;

            let top_menu_x = 30.0;
            let top_menu_y = top_menu_btn_y + top_menu_btn_h + 5.0;

            // Draw top-left menu toggle button
            let top_btn_rounded = kurbo::RoundedRect::new(top_menu_btn_x, top_menu_btn_y, top_menu_btn_x + top_menu_btn_w, top_menu_btn_y + top_menu_btn_h, 8.0);
            let top_btn_bg_color = vello::peniko::Color::from_rgb8(
                if hover_state == 30 { 70 } else { 25 },
                if hover_state == 30 { 70 } else { 25 },
                if hover_state == 30 { 70 } else { 25 },
            );
            scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, top_btn_bg_color, None, &top_btn_rounded);
            scene.stroke(&stroke_style, kurbo::Affine::IDENTITY, border_color, None, &top_btn_rounded);

            self.draw_text_to_scene(scene, "Menu", top_menu_btn_x + 24.0, top_menu_btn_y + 44.0, 36.0, font, vello::peniko::Color::WHITE);

            if self.top_menu_open {
                if self.recent_menu_open {
                    let recents = load_recent_files();
                    let item_count = recents.len() + 1;
                    let dyn_menu_h = (item_count as f64 * 50.0) + 20.0;
                    let dyn_menu_w = 500.0;
                    
                    let top_menu_rounded = kurbo::RoundedRect::new(top_menu_x, top_menu_y, top_menu_x + dyn_menu_w, top_menu_y + dyn_menu_h, 8.0);
                    scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, bg_color, None, &top_menu_rounded);
                    scene.stroke(&stroke_style, kurbo::Affine::IDENTITY, border_color, None, &top_menu_rounded);
                    
                    let back_y = top_menu_y + 10.0;
                    if hover_state == 35 {
                        let h_rect = kurbo::Rect::new(top_menu_x + 1.0, back_y, top_menu_x + dyn_menu_w - 1.0, back_y + 45.0);
                        scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, vello::peniko::Color::from_rgba8(100, 100, 100, 200), None, &h_rect);
                    }
                    let mut arrow_path = kurbo::BezPath::new();
                    // Horizontal line
                    arrow_path.move_to((top_menu_x + 20.0, back_y + 22.0));
                    arrow_path.line_to((top_menu_x + 36.0, back_y + 22.0));
                    // Top slant
                    arrow_path.move_to((top_menu_x + 20.0, back_y + 22.0));
                    arrow_path.line_to((top_menu_x + 28.0, back_y + 14.0));
                    // Bottom slant
                    arrow_path.move_to((top_menu_x + 20.0, back_y + 22.0));
                    arrow_path.line_to((top_menu_x + 28.0, back_y + 30.0));
                    
                    let arrow_stroke = kurbo::Stroke::new(2.5)
                        .with_caps(kurbo::Cap::Round)
                        .with_join(kurbo::Join::Round);
                    scene.stroke(&arrow_stroke, kurbo::Affine::IDENTITY, vello::peniko::Color::WHITE, None, &arrow_path);
                    
                    self.draw_text_to_scene(scene, "Volver", top_menu_x + 48.0, back_y + 32.0, 24.0, font, vello::peniko::Color::WHITE);
                    
                    for (idx, file) in recents.iter().enumerate() {
                        let item_y = back_y + 50.0 + (idx as f64 * 50.0);
                        let h_state = 36 + idx as u8;
                        if hover_state == h_state {
                            let h_rect = kurbo::Rect::new(top_menu_x + 1.0, item_y, top_menu_x + dyn_menu_w - 1.0, item_y + 45.0);
                            scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, vello::peniko::Color::from_rgba8(100, 100, 100, 200), None, &h_rect);
                        }
                        let display_name = std::path::Path::new(file).file_name().and_then(|n| n.to_str()).unwrap_or(file);
                        let max_text_width = dyn_menu_w - 40.0;
                        let elided_name = self.truncate_text_with_ellipsis(display_name, max_text_width, 24.0, font);
                        self.draw_text_to_scene(scene, &elided_name, top_menu_x + 20.0, item_y + 32.0, 24.0, font, vello::peniko::Color::WHITE);
                    }
                } else {
                    let dyn_menu_w = 320.0;
                    let dyn_menu_h = 220.0;
                    
                    let top_menu_rounded = kurbo::RoundedRect::new(top_menu_x, top_menu_y, top_menu_x + dyn_menu_w, top_menu_y + dyn_menu_h, 8.0);
                    scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, bg_color, None, &top_menu_rounded);
                    scene.stroke(&stroke_style, kurbo::Affine::IDENTITY, border_color, None, &top_menu_rounded);
                    
                    let item1_y = top_menu_y + 10.0;
                    if hover_state == 31 {
                        let h_rect = kurbo::Rect::new(top_menu_x + 1.0, item1_y, top_menu_x + dyn_menu_w - 1.0, item1_y + 45.0);
                        scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, vello::peniko::Color::from_rgba8(100, 100, 100, 200), None, &h_rect);
                    }
                    self.draw_text_to_scene(scene, "Abrir archivo...", top_menu_x + 20.0, item1_y + 32.0, 28.0, font, vello::peniko::Color::WHITE);
                    
                    let item2_y = item1_y + 50.0;
                    if hover_state == 32 {
                        let h_rect = kurbo::Rect::new(top_menu_x + 1.0, item2_y, top_menu_x + dyn_menu_w - 1.0, item2_y + 45.0);
                        scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, vello::peniko::Color::from_rgba8(100, 100, 100, 200), None, &h_rect);
                    }
                    self.draw_text_to_scene(scene, "Archivos recientes", top_menu_x + 20.0, item2_y + 32.0, 28.0, font, vello::peniko::Color::WHITE);

                    let item3_y = item2_y + 50.0;
                    if hover_state == 33 {
                        let h_rect = kurbo::Rect::new(top_menu_x + 1.0, item3_y, top_menu_x + dyn_menu_w - 1.0, item3_y + 45.0);
                        scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, vello::peniko::Color::from_rgba8(100, 100, 100, 200), None, &h_rect);
                    }
                    self.draw_text_to_scene(scene, "Lecturas", top_menu_x + 20.0, item3_y + 32.0, 28.0, font, vello::peniko::Color::WHITE);

                    let item4_y = item3_y + 50.0;
                    if hover_state == 34 {
                        let h_rect = kurbo::Rect::new(top_menu_x + 1.0, item4_y, top_menu_x + dyn_menu_w - 1.0, item4_y + 45.0);
                        scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, vello::peniko::Color::from_rgba8(100, 100, 100, 200), None, &h_rect);
                    }
                    self.draw_text_to_scene(scene, "Salir", top_menu_x + 20.0, item4_y + 32.0, 28.0, font, vello::peniko::Color::WHITE);
                }
            }
        }

        if !has_pdf {
            return; // Don't draw the rest of the UI overlays
        }

        let overlay_width = 504.0;
        let overlay_height = 100.0;
        let overlay_x = width - overlay_width - 30.0;
        let overlay_y = height - overlay_height - 30.0;

        let stroke_style = kurbo::Stroke::new(2.0);
        let border_color = vello::peniko::Color::from_rgb8(100, 100, 100);
        let bg_color = vello::peniko::Color::from_rgba8(25, 25, 25, 220);

        // 1. Draw bottom-right zoom overlay
        let zoom_rounded = kurbo::RoundedRect::new(overlay_x, overlay_y, overlay_x + overlay_width, overlay_y + overlay_height, 8.0);
        scene.fill(
            vello::peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            bg_color,
            None,
            &zoom_rounded,
        );
        scene.stroke(
            &stroke_style,
            kurbo::Affine::IDENTITY,
            border_color,
            None,
            &zoom_rounded,
        );

        let btn_y_local = overlay_y + 12.0;
        let btn_size = 76.0;

        let draw_btn = |scene: &mut Scene, bx: f64, label: &str, hovered: bool| {
            let btn_bg = vello::peniko::Color::from_rgb8(if hovered { 70 } else { 40 }, if hovered { 70 } else { 40 }, if hovered { 70 } else { 40 });
            let btn_rect = kurbo::Rect::new(bx, btn_y_local, bx + btn_size, btn_y_local + btn_size);
            scene.fill(
                vello::peniko::Fill::NonZero,
                kurbo::Affine::IDENTITY,
                btn_bg,
                None,
                &btn_rect,
            );
            let tw = self.measure_text_width(label, 36.0, font) as f64;
            self.draw_text_to_scene(
                scene,
                label,
                bx + (btn_size - tw) / 2.0,
                btn_y_local + 52.0,
                36.0,
                font,
                vello::peniko::Color::WHITE,
            );
        };

        draw_btn(scene, overlay_x + 20.0, "-", hover_state == 2);
        draw_btn(scene, overlay_x + 306.0, "+", hover_state == 3);
        draw_btn(scene, overlay_x + 408.0, "R", hover_state == 4);

        let current_fit_zoom = self.calculate_fit_height_zoom(height as u32);
        let zoom_pct = format!("{:.0}%", (self.zoom / current_fit_zoom) * 100.0);
        let lw = self.measure_text_width(&zoom_pct, 32.0, font) as f64;
        let lx = overlay_x + 96.0 + (210.0 - lw) / 2.0;
        self.draw_text_to_scene(
            scene,
            &zoom_pct,
            lx,
            btn_y_local + 48.0,
            32.0,
            font,
            vello::peniko::Color::WHITE,
        );

        // 2. Draw bottom-center pagination overlay
        let pag_overlay_width = 372.0;
        let pag_overlay_height = 100.0;
        let pag_overlay_x = (width - pag_overlay_width) / 2.0;
        let pag_overlay_y = height - pag_overlay_height - 30.0;

        let pag_rounded = kurbo::RoundedRect::new(pag_overlay_x, pag_overlay_y, pag_overlay_x + pag_overlay_width, pag_overlay_y + pag_overlay_height, 8.0);
        scene.fill(
            vello::peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            bg_color,
            None,
            &pag_rounded,
        );
        scene.stroke(
            &stroke_style,
            kurbo::Affine::IDENTITY,
            border_color,
            None,
            &pag_rounded,
        );

        let p_btn_y_local = pag_overlay_y + 12.0;

        let draw_nav_btn = |scene: &mut Scene, bx: f64, direction_left: bool, hovered: bool| {
            let btn_bg = vello::peniko::Color::from_rgb8(if hovered { 70 } else { 40 }, if hovered { 70 } else { 40 }, if hovered { 70 } else { 40 });
            let btn_rect = kurbo::Rect::new(bx, p_btn_y_local, bx + btn_size, p_btn_y_local + btn_size);
            scene.fill(
                vello::peniko::Fill::NonZero,
                kurbo::Affine::IDENTITY,
                btn_bg,
                None,
                &btn_rect,
            );

            let mut pb = kurbo::BezPath::new();
            if direction_left {
                pb.move_to((bx + 42.5, p_btn_y_local + 24.0));
                pb.line_to((bx + 28.5, p_btn_y_local + 38.0));
                pb.line_to((bx + 42.5, p_btn_y_local + 52.0));
                pb.line_to((bx + 47.5, p_btn_y_local + 52.0));
                pb.line_to((bx + 33.5, p_btn_y_local + 38.0));
                pb.line_to((bx + 47.5, p_btn_y_local + 24.0));
            } else {
                pb.move_to((bx + 33.5, p_btn_y_local + 24.0));
                pb.line_to((bx + 47.5, p_btn_y_local + 38.0));
                pb.line_to((bx + 33.5, p_btn_y_local + 52.0));
                pb.line_to((bx + 28.5, p_btn_y_local + 52.0));
                pb.line_to((bx + 42.5, p_btn_y_local + 38.0));
                pb.line_to((bx + 28.5, p_btn_y_local + 24.0));
            }
            pb.close_path();
            scene.fill(
                vello::peniko::Fill::NonZero,
                kurbo::Affine::IDENTITY,
                vello::peniko::Color::WHITE,
                None,
                &pb,
            );
        };

        draw_nav_btn(scene, pag_overlay_x + 12.0, true, hover_state == 21);
        draw_nav_btn(scene, pag_overlay_x + 284.0, false, hover_state == 22);

        // Editable page input box background
        let input_x = pag_overlay_x + 100.0;
        let input_w = 80.0;
        let input_bg = vello::peniko::Color::from_rgb8(
            if self.page_input_active { 35 } else if hover_state == 23 { 30 } else { 15 },
            if self.page_input_active { 35 } else if hover_state == 23 { 30 } else { 15 },
            if self.page_input_active { 35 } else if hover_state == 23 { 30 } else { 15 },
        );
        let input_border = vello::peniko::Color::from_rgb8(
            if self.page_input_active { 100 } else { 100 },
            if self.page_input_active { 180 } else { 100 },
            if self.page_input_active { 255 } else { 100 },
        );

        let input_rect = kurbo::Rect::new(input_x, p_btn_y_local, input_x + input_w, p_btn_y_local + btn_size);
        scene.fill(
            vello::peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            input_bg,
            None,
            &input_rect,
        );
        scene.stroke(
            &kurbo::Stroke::new(2.0),
            kurbo::Affine::IDENTITY,
            input_border,
            None,
            &input_rect,
        );

        let page_text = if self.page_input_active {
            self.page_input_text.clone()
        } else {
            (self.get_current_page_idx() + 1).to_string()
        };
        let mut text_to_draw = page_text;
        if self.page_input_active {
            text_to_draw.push('|');
        }

        let lw = self.measure_text_width(&text_to_draw, 32.0, font) as f64;
        let lx = input_x + (input_w - lw) / 2.0;
        self.draw_text_to_scene(
            scene,
            &text_to_draw,
            lx,
            p_btn_y_local + 48.0,
            32.0,
            font,
            vello::peniko::Color::WHITE,
        );

        let total_text = format!("/ {}", self.pages.len());
        let total_x = pag_overlay_x + 192.0;
        let total_w = 80.0;
        let tw = self.measure_text_width(&total_text, 32.0, font) as f64;
        let t_lx = total_x + (total_w - tw) / 2.0;
        self.draw_text_to_scene(
            scene,
            &total_text,
            t_lx,
            p_btn_y_local + 48.0,
            32.0,
            font,
            vello::peniko::Color::from_rgb8(180, 180, 180),
        );

        // 3. Left Menu Overlay (Bottom-Left)
        let menu_btn_x = 30.0;
        let menu_btn_y = height - 100.0 - 30.0;
        let menu_btn_w = 84.0;
        let menu_btn_h = 100.0;

        let menu_w = 480.0;
        let menu_h = 560.0;
        let menu_x = 30.0;
        let menu_y = height - 100.0 - 30.0 - menu_h - 10.0;

        if hover_state >= 9 || self.left_menu_open {
            // Draw menu toggle button
            let btn_rounded = kurbo::RoundedRect::new(menu_btn_x, menu_btn_y, menu_btn_x + menu_btn_w, menu_btn_y + menu_btn_h, 8.0);
            let btn_bg_color = vello::peniko::Color::from_rgb8(
                if hover_state == 9 { 70 } else { 25 },
                if hover_state == 9 { 70 } else { 25 },
                if hover_state == 9 { 70 } else { 25 },
            );
            scene.fill(
                vello::peniko::Fill::NonZero,
                kurbo::Affine::IDENTITY,
                btn_bg_color,
                None,
                &btn_rounded,
            );
            scene.stroke(
                &stroke_style,
                kurbo::Affine::IDENTITY,
                border_color,
                None,
                &btn_rounded,
            );

            // Three lines for hamburger menu
            for i in 0..3 {
                let rect = kurbo::Rect::new(
                    menu_btn_x + 21.0,
                    menu_btn_y + 34.0 + (i as f64 * 12.0),
                    menu_btn_x + 21.0 + 42.0,
                    menu_btn_y + 34.0 + (i as f64 * 12.0) + 6.0,
                );
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    kurbo::Affine::IDENTITY,
                    vello::peniko::Color::WHITE,
                    None,
                    &rect,
                );
            }

            if self.left_menu_open {
                let menu_rounded = kurbo::RoundedRect::new(menu_x, menu_y, menu_x + menu_w, menu_y + menu_h, 8.0);
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    kurbo::Affine::IDENTITY,
                    bg_color,
                    None,
                    &menu_rounded,
                );
                scene.stroke(
                    &stroke_style,
                    kurbo::Affine::IDENTITY,
                    border_color,
                    None,
                    &menu_rounded,
                );

                let items = [
                    "Single-page view",
                    "Two-page view",
                    "Enable scrolling",
                    "Actual size",
                    "Zoom to page level",
                    "Fit to width",
                    "Fit height",
                    "Fit visible content",
                ];
                let item_h = menu_h / 8.0;
                for (i, text) in items.iter().enumerate() {
                    let iy = menu_y + (i as f64 * item_h);
                    if hover_state == 10 + i as u8 {
                        let h_rect = kurbo::Rect::new(menu_x + 1.0, iy + 1.0, menu_x + menu_w - 1.0, iy + item_h - 1.0);
                        scene.fill(
                            vello::peniko::Fill::NonZero,
                            kurbo::Affine::IDENTITY,
                            vello::peniko::Color::from_rgba8(100, 100, 100, 200),
                            None,
                            &h_rect,
                        );
                    }
                    self.draw_text_to_scene(
                        scene,
                        text,
                        menu_x + 20.0,
                        iy + 44.0,
                        32.0,
                        font,
                        vello::peniko::Color::WHITE,
                    );
                }
            }
        }



        if let Some(t) = self.loading_timeout {
            if t.elapsed().as_secs_f32() <= 1.5 {
                let bg_rect = kurbo::Rect::new(0.0, 0.0, width, height);
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    kurbo::Affine::IDENTITY,
                    vello::peniko::Color::from_rgba8(0, 0, 0, 180),
                    None,
                    &bg_rect,
                );
                let title = "Cargando documento...";
                let title_size = 48.0;
                let tw = self.measure_text_width(title, title_size as f32, font) as f64;
                let tx = (width - tw) / 2.0;
                let ty = height / 2.0;
                self.draw_text_to_scene(scene, title, tx, ty, title_size, font, vello::peniko::Color::WHITE);
            }
        }
    }

    fn draw_scrollbar_to_scene(&self, scene: &mut Scene, width: f64, height: f64) {
        let total_h_scrollbar = self.pages.last().map(|p| p.top_y + p.height).unwrap_or(0.0) * self.zoom;
        let min_scroll = -(total_h_scrollbar - height as f32 + 100.0).max(0.0);
        let scroll_range = 100.0 - min_scroll;
        
        if scroll_range > 0.0 {
            let track_w = 20.0;
            let track_x = width - track_w - 10.0;
            let track_y = 20.0;
            let track_h = height - 40.0;

            let track_rect = kurbo::RoundedRect::new(
                track_x, track_y, track_x + track_w, track_y + track_h, 8.0
            );
            scene.fill(
                vello::peniko::Fill::NonZero,
                kurbo::Affine::IDENTITY,
                vello::peniko::Color::from_rgba8(30, 30, 30, 150),
                None,
                &track_rect,
            );

            let thumb_h = ((height as f32 / (total_h_scrollbar + 100.0)) * track_h as f32).clamp(40.0, track_h as f32) as f64;
            let scroll_pct = (100.0 - self.scroll_y) / scroll_range;
            
            let thumb_y = track_y + (scroll_pct as f64) * (track_h - thumb_h);
            let thumb_rect = kurbo::RoundedRect::new(
                track_x, thumb_y, track_x + track_w, thumb_y + thumb_h, 8.0
            );
            
            let thumb_color = if self.is_dragging_scrollbar {
                vello::peniko::Color::from_rgba8(180, 180, 180, 255)
            } else {
                vello::peniko::Color::from_rgba8(120, 120, 120, 200)
            };
            
            scene.fill(
                vello::peniko::Fill::NonZero,
                kurbo::Affine::IDENTITY,
                thumb_color,
                None,
                &thumb_rect,
            );
        }
    }

    fn update_scroll_from_mouse_y(&mut self, my: f32) {
        let height = self.window_size.height as f32;
        let track_y = 20.0;
        let track_h = height - 40.0;
        let total_h_scrollbar = self.pages.last().map(|p| p.top_y + p.height).unwrap_or(0.0) * self.zoom;
        let thumb_h = ((height / (total_h_scrollbar + 100.0)) * track_h).clamp(40.0, track_h);
        
        let movable_range = track_h - thumb_h;
        if movable_range <= 0.0 { return; }
        
        let relative_y = (my - track_y - thumb_h / 2.0).clamp(0.0, movable_range);
        let scroll_pct = relative_y / movable_range;
        
        let min_scroll = -(total_h_scrollbar - height + 100.0).max(0.0);
        let scroll_range = 100.0 - min_scroll;
        
        self.scroll_y = 100.0 - (scroll_pct * scroll_range);
        self.target_scroll_y = self.scroll_y;
        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
    }

    fn queue_page_requests(&self, _width: u32, height: u32) {
        let page_count = self.pages.len();
        if page_count == 0 { return; }
        let epoch = self.render_epoch.load(Ordering::SeqCst);
        let zoom_key = (self.zoom * 1000.0) as u32;

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
            if page_y >= height as i32 && page_idx > first_visible {
                next_non_visible = page_idx;
                break;
            }
        }
        println!("queue_page_requests: page_count={}, first_visible={}, next_non_visible={}", page_count, first_visible, next_non_visible);

        // Send high priority requests for visible pages
        {
            let mut requested = self.requested_pages.borrow_mut();
            let images = self.page_images.borrow();
            for idx in first_visible..next_non_visible {
                let needs_render = match images.get(&idx) {
                    Some(cached) => (cached.zoom - self.zoom).abs() > 0.001,
                    None => true,
                };
                if needs_render && !requested.contains(&(idx, zoom_key)) {
                    println!("Requesting render for page {}", idx);
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

        // Preload
        {
            let (preload_above, preload_below) = if self.scroll_down_direction {
                (1, 3)
            } else {
                (3, 1)
            };
            let mut requested = self.requested_pages.borrow_mut();
            let images = self.page_images.borrow();

            let start_above = first_visible.saturating_sub(preload_above);
            for idx in start_above..first_visible {
                let needs_render = match images.get(&idx) {
                    Some(cached) => (cached.zoom - self.zoom).abs() > 0.001,
                    None => true,
                };
                if needs_render && !requested.contains(&(idx, zoom_key)) {
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
                let needs_render = match images.get(&idx) {
                    Some(cached) => (cached.zoom - self.zoom).abs() > 0.001,
                    None => true,
                };
                if needs_render && !requested.contains(&(idx, zoom_key)) {
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

    #[allow(dead_code)]
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

    fn get_hover_state(&self, mx: f32, my: f32) -> u8 {
        let width = self.window_size.width as f32;
        let height = self.window_size.height as f32;

        if !self.pdf_path.is_empty() {
            let track_w = 20.0;
            let track_x = width - track_w - 10.0;
            let track_y = 20.0;
            let track_h = height - 40.0;
            let total_h_scrollbar = self.pages.last().map(|p| p.top_y + p.height).unwrap_or(0.0) * self.zoom;
            let min_scroll = -(total_h_scrollbar - height + 100.0).max(0.0);
            let scroll_range = 100.0 - min_scroll;
            
            if scroll_range > 0.0 && mx >= track_x && mx <= track_x + track_w && my >= track_y && my <= track_y + track_h {
                return 60;
            }
        }

        if self.view_lecturas {
            if mx >= 30.0 && mx <= 190.0 && my >= 30.0 && my <= 94.0 {
                return 50; // Volver button
            }
            
            let search_w = 600.0;
            let search_h = 60.0;
            let search_x = (width - search_w) / 2.0;
            let search_y = 160.0;
            if mx >= search_x && mx <= search_x + search_w && my >= search_y && my <= search_y + search_h {
                return 53; // Search bar
            }
            
            let pag_y = height - 80.0;
            if mx >= width / 2.0 - 150.0 && mx <= width / 2.0 - 50.0 && my >= pag_y && my <= pag_y + 50.0 {
                return 51; // Prev button
            }
            if mx >= width / 2.0 + 50.0 && mx <= width / 2.0 + 150.0 && my >= pag_y && my <= pag_y + 50.0 {
                return 52; // Next button
            }

            let start_y = 260.0;
            let item_h = 140.0;
            let gap = 20.0;
            let my_rel = my - start_y;
            if my_rel >= 0.0 {
                let idx = (my_rel / (item_h + gap)).floor() as i32;
                let btn_y = start_y + (idx as f32 * (item_h + gap)) + 40.0;
                
                let list_w = (width - 100.0).max(800.0);
                let list_x = (width - list_w) / 2.0;
                let btn_w = 180.0;
                let btn_x = list_x + list_w - btn_w - 30.0;

                if mx >= btn_x && mx <= btn_x + btn_w && my >= btn_y && my <= btn_y + 60.0 {
                    return 100 + idx as u8;
                }
            }
            return 0;
        }

        let has_pdf = !self.pages.is_empty();

        // Top-left menu (Abrir Archivo) - only active when no PDF is loaded
        if !has_pdf {
            let top_menu_btn_x = 30.0;
            let top_menu_btn_y = 30.0;
            let top_menu_btn_w = 160.0;
            let top_menu_btn_h = 64.0;

            if self.top_menu_open {
                let top_menu_x = 30.0;
                let top_menu_y = top_menu_btn_y + top_menu_btn_h + 5.0;

                if self.recent_menu_open {
                    let recents = load_recent_files();
                    let item_count = recents.len() + 1;
                    let dyn_menu_h = (item_count as f32 * 50.0) + 20.0;
                    let dyn_menu_w = 500.0;

                    if mx >= top_menu_x && mx <= top_menu_x + dyn_menu_w && my >= top_menu_y && my <= top_menu_y + dyn_menu_h {
                        let relative_y = my - top_menu_y - 10.0;
                        if relative_y >= 0.0 {
                            let idx = (relative_y / 50.0).floor() as i32;
                            if idx == 0 {
                                return 35; // Back button
                            } else if idx > 0 && idx <= recents.len() as i32 {
                                return 36 + (idx - 1) as u8; // File item
                            }
                        }
                    }
                } else {
                    let dyn_menu_w = 320.0;
                    let dyn_menu_h = 220.0;

                    if mx >= top_menu_x && mx <= top_menu_x + dyn_menu_w && my >= top_menu_y && my <= top_menu_y + dyn_menu_h {
                        let relative_y = my - top_menu_y - 5.0;
                        if relative_y >= 0.0 {
                            let idx = (relative_y / 50.0).floor() as i32;
                            if idx == 0 {
                                return 31; // "Abrir archivo..."
                            } else if idx == 1 {
                                return 32; // "Open recent files"
                            } else if idx == 2 {
                                return 33; // "Exit"
                            } else if idx == 3 {
                                return 34;
                            }
                        }
                    }
                }
            }

            if mx >= top_menu_btn_x && mx <= top_menu_btn_x + top_menu_btn_w && my >= top_menu_btn_y && my <= top_menu_btn_y + top_menu_btn_h {
                return 30; // Top Menu toggle button
            }
        }

        if !has_pdf {
            return 0;
        }

        // Bottom-left menu (Hamburger)
        let menu_btn_x = 30.0;
        let menu_btn_y = height - 100.0 - 30.0;
        let menu_btn_w = 84.0;
        let menu_btn_h = 100.0;

        if self.left_menu_open {
            let menu_w = 480.0;
            let menu_h = 560.0;
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
        let width = self.window_size.width.max(1);
        let height = self.window_size.height.max(1);
        
        let size_changed = self.surface_size.0 != width || self.surface_size.1 != height;
        if size_changed {
            if let Some(render_surface) = self.render_surface.as_mut() {
                self.render_cx.resize_surface(render_surface, width, height);
                self.surface_size = (width, height);
            }
        }

        // Detect scroll direction
        let scroll_diff = self.target_scroll_y - self.last_scroll_y;
        if scroll_diff < -0.1 {
            self.scroll_down_direction = true;
        } else if scroll_diff > 0.1 {
            self.scroll_down_direction = false;
        }
        self.last_scroll_y = self.target_scroll_y;

        // No easing
        self.scroll_x = self.target_scroll_x;
        self.scroll_y = self.target_scroll_y;

        let surface_texture = match self.render_surface.as_ref().unwrap().surface.get_current_texture() {
            vello::wgpu::CurrentSurfaceTexture::Success(t) => t,
            _ => return,
        };

        let mut scene = vello::Scene::new();

        // 1. Draw Gray Background
        let bg_rect = kurbo::Rect::new(0.0, 0.0, width as f64, height as f64);
        scene.fill(
            vello::peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            vello::peniko::Color::from_rgb8(82, 86, 89),
            None,
            &bg_rect,
        );

        let page_count = self.pages.len();
        let is_loading = self.page_images.borrow().is_empty();

        if self.view_lecturas {
            self.draw_lecturas_to_scene(&mut scene, width as f64, height as f64, self.get_hover_state(self.mouse_pos.0, self.mouse_pos.1));
            
            let render_surface = self.render_surface.as_mut().unwrap();
            let device = &self.render_cx.devices[render_surface.dev_id].device;
            let queue = &self.render_cx.devices[render_surface.dev_id].queue;
            let renderer = self.renderer.as_mut().unwrap();

            renderer.render_to_texture(
                device,
                queue,
                &scene,
                &render_surface.target_view,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::from_rgb8(18, 18, 18),
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            ).unwrap();
            {
                let mut encoder = device.create_command_encoder(&vello::wgpu::CommandEncoderDescriptor { label: Some("Blit") });
                render_surface.blitter.copy(
                    device,
                    &mut encoder,
                    &render_surface.target_view,
                    &surface_texture.texture.create_view(&vello::wgpu::TextureViewDescriptor::default()),
                );
                queue.submit(std::iter::once(encoder.finish()));
            }
            let _ = device.poll(vello::wgpu::PollType::Poll);
            surface_texture.present();
            if let Some(w) = self.window.as_ref() { w.set_visible(true); }
            return;
        }

        // 2. Draw Splash Screen if loading
        if page_count > 0 && is_loading {
            self.draw_splash_screen_to_scene(&mut scene, width as f64, height as f64);
            
            // Queue up initial page requests
            self.queue_page_requests(width, height);

            let render_surface = self.render_surface.as_mut().unwrap();
            let device = &self.render_cx.devices[render_surface.dev_id].device;
            let queue = &self.render_cx.devices[render_surface.dev_id].queue;
            let renderer = self.renderer.as_mut().unwrap();
            renderer.render_to_texture(
                device,
                queue,
                &scene,
                &render_surface.target_view,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::from_rgb8(18, 18, 18),
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            ).unwrap();
            {
                let mut encoder = device.create_command_encoder(&vello::wgpu::CommandEncoderDescriptor { label: Some("Blit") });
                render_surface.blitter.copy(
                    device,
                    &mut encoder,
                    &render_surface.target_view,
                    &surface_texture.texture.create_view(&vello::wgpu::TextureViewDescriptor::default()),
                );
                queue.submit(std::iter::once(encoder.finish()));
            }
            let _ = device.poll(vello::wgpu::PollType::Poll);
            surface_texture.present();
            // Do NOT make window visible here yet to avoid flash. Wait until it's fully loaded.
            return;
        }

        if page_count == 0 {
            self.draw_welcome_screen_to_scene(&mut scene, width as f64, height as f64);
            self.draw_ui_overlays_to_scene(&mut scene, width as f64, height as f64, self.get_hover_state(self.mouse_pos.0, self.mouse_pos.1));

            let render_surface = self.render_surface.as_mut().unwrap();
            let device = &self.render_cx.devices[render_surface.dev_id].device;
            let queue = &self.render_cx.devices[render_surface.dev_id].queue;
            let renderer = self.renderer.as_mut().unwrap();

            renderer.render_to_texture(
                device,
                queue,
                &scene,
                &render_surface.target_view,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::from_rgb8(18, 18, 18),
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            ).unwrap();
            {
                let mut encoder = device.create_command_encoder(&vello::wgpu::CommandEncoderDescriptor { label: Some("Blit") });
                render_surface.blitter.copy(
                    device,
                    &mut encoder,
                    &render_surface.target_view,
                    &surface_texture.texture.create_view(&vello::wgpu::TextureViewDescriptor::default()),
                );
                queue.submit(std::iter::once(encoder.finish()));
            }
            let _ = device.poll(vello::wgpu::PollType::Poll);
            surface_texture.present();
            if let Some(w) = self.window.as_ref() { 
                w.set_maximized(true);
                w.set_visible(true); 
            }
            return;
        }

        // 3. Draw Rendered Page Scenes
        let images = self.page_images.borrow();
        for i in 0..page_count {
            let page = &self.pages[i];
            let page_h = page.height * self.zoom;
            let page_w = page.width * self.zoom;
            let page_y = self.scroll_y + page.top_y * self.zoom;
            let page_x = (width as f32 / 2.0) + (page.center_x_offset * self.zoom) - (page_w / 2.0);

            if page_y + page_h > 0.0 && page_y < height as f32 {
                if let Some(cached) = images.get(&i) {
                    let transform = kurbo::Affine::translate((page_x as f64, page_y as f64));
                    scene.append(&cached.scene, Some(transform));
                } else {
                    let rect = kurbo::Rect::new(
                        page_x as f64,
                        page_y as f64,
                        (page_x + page_w) as f64,
                        (page_y + page_h) as f64,
                    );
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        kurbo::Affine::IDENTITY,
                        vello::peniko::Color::WHITE,
                        None,
                        &rect,
                    );
                }
            }
        }

        // 4. Draw UI Overlays (Zoom, Pagination, Left Menu)
        let hover_state = self.get_hover_state(self.mouse_pos.0, self.mouse_pos.1);
        let show_overlays = hover_state > 0 || self.page_input_active;
        if show_overlays {
            self.draw_ui_overlays_to_scene(&mut scene, width as f64, height as f64, hover_state);
        }
        
        // 4.5 Draw Scrollbar
        self.draw_scrollbar_to_scene(&mut scene, width as f64, height as f64);

        // 5. Send pre-fetch requests
        self.queue_page_requests(width, height);

        // 6. Submit Render commands to GPU
        let render_surface = self.render_surface.as_mut().unwrap();
        let device = &self.render_cx.devices[render_surface.dev_id].device;
        let queue = &self.render_cx.devices[render_surface.dev_id].queue;
        let renderer = self.renderer.as_mut().unwrap();
        renderer.render_to_texture(
            device,
            queue,
            &scene,
            &render_surface.target_view,
            &vello::RenderParams {
                base_color: vello::peniko::Color::from_rgb8(82, 86, 89),
                width,
                height,
                antialiasing_method: vello::AaConfig::Area,
            },
        ).unwrap();
        {
            let mut encoder = device.create_command_encoder(&vello::wgpu::CommandEncoderDescriptor { label: Some("Blit") });
            render_surface.blitter.copy(
                device,
                &mut encoder,
                &render_surface.target_view,
                &surface_texture.texture.create_view(&vello::wgpu::TextureViewDescriptor::default()),
            );
            queue.submit(std::iter::once(encoder.finish()));
        }
        surface_texture.present();
        if let Some(w) = self.window.as_ref() { 
            w.set_maximized(true);
            w.set_visible(true); 
        }

        // Keep requesting redraws if we are currently loading something to animate the spinners
        if let Some(t) = self.loading_timeout {
            if t.elapsed().as_secs_f32() <= 1.5 {
                _window.request_redraw();
            } else {
                self.loading_timeout = None;
                self.loading_lectura_idx = None;
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window = match event_loop.create_window(
                Window::default_attributes()
                    .with_title("Adobo Reader")
                    .with_visible(false)
            ) {
                Ok(w) => Arc::new(w),
                Err(e) => {
                    eprintln!("Failed to create window: {}", e);
                    return;
                }
            };
            let size = window.inner_size();
            self.window_size = size;

            // Initialize RenderSurface
            let render_surface = pollster::block_on(self.render_cx.create_surface(
                window.clone(),
                size.width.max(1),
                size.height.max(1),
                vello::wgpu::PresentMode::Fifo,
            )).expect("Failed to create render surface");
            
            self.renderer = Some(vello::Renderer::new(
                &self.render_cx.devices[render_surface.dev_id].device,
                vello::RendererOptions {
                    use_cpu: false,
                    antialiasing_support: vello::AaSupport::all(),
                    num_init_threads: std::num::NonZeroUsize::new(1),
                    pipeline_cache: None,
                },
            ).expect("Failed to create Renderer"));
            self.render_surface = Some(render_surface);
            self.surface_size = (size.width.max(1), size.height.max(1));
            self.last_draw_time = std::time::Instant::now();
            
            if let Some(icon) = self.window_icon.clone() {
                window.set_window_icon(Some(icon));
            }
            
            self.window = Some(window.clone());

            if let Some(pid) = self.kill_pid {
                std::process::Command::new("taskkill")
                    .arg("/F")
                    .arg("/PID")
                    .arg(pid.to_string())
                    .spawn()
                    .ok();
            }

            if size.width > 0 && size.height > 0 {
                let initial_height = if let Some(monitor) = window.current_monitor().or_else(|| window.primary_monitor()) {
                    monitor.size().height.saturating_sub(60)
                } else {
                    size.height
                };
                self.zoom = self.calculate_fit_height_zoom(initial_height);
                self.rendered_zoom = self.zoom;
                self.clear_cache();
                self.center_on_content(size.width, size.height);
                self.zoom_initialized = true;
                if !self.pdf_path.is_empty() {
                    if let Some(prog) = crate::db::get_progress(&self.pdf_path) {
                        self.jump_to_page(prog.current_page as usize);
                    }
                }
            }
            self.window = Some(window.clone());
            self.draw(&window);
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if !self.pdf_path.is_empty() && !self.pages.is_empty() {
                    let progress = crate::db::ReadingProgress {
                        title: self.pdf_title.clone(),
                        author: self.pdf_author.clone(),
                        current_page: self.get_current_page_idx() as u32,
                        total_pages: self.pages.len() as u32,
                    };
                    let _ = crate::db::save_progress(&self.pdf_path, &progress);
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.window_size = size;
                if !self.zoom_initialized && size.width > 0 && size.height > 0 {
                    self.zoom = self.calculate_fit_height_zoom(size.height);
                    self.rendered_zoom = self.zoom;
                    self.clear_cache();
                    self.center_on_content(size.width, size.height);
                    self.zoom_initialized = true;
                    if !self.pdf_path.is_empty() {
                        if let Some(prog) = crate::db::get_progress(&self.pdf_path) {
                            self.jump_to_page(prog.current_page as usize);
                        }
                    }
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let new_mouse_pos = (position.x as f32, position.y as f32);
                if self.is_dragging_scrollbar {
                    self.update_scroll_from_mouse_y(new_mouse_pos.1);
                }
                let old_state = self.get_hover_state(self.mouse_pos.0, self.mouse_pos.1);
                let new_state = self.get_hover_state(new_mouse_pos.0, new_mouse_pos.1);
                self.mouse_pos = new_mouse_pos;
                if old_state != new_state {
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                        if new_state > 0 {
                            window.set_cursor(winit::window::CursorIcon::Pointer);
                        } else {
                            window.set_cursor(winit::window::CursorIcon::Default);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state: winit::event::ElementState::Pressed, button: winit::event::MouseButton::Left, .. } => {
                let hover = self.get_hover_state(self.mouse_pos.0, self.mouse_pos.1);
                
                if hover == 60 {
                    self.is_dragging_scrollbar = true;
                    self.update_scroll_from_mouse_y(self.mouse_pos.1);
                    return;
                }

                if self.view_lecturas {
                    if hover == 50 {
                        self.view_lecturas = false;
                        self.lecturas_search_active = false;
                        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                    } else if hover == 51 {
                        self.lecturas_page_idx = self.lecturas_page_idx.saturating_sub(1);
                        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                    } else if hover == 52 {
                        self.lecturas_page_idx += 1;
                        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                    } else if hover == 53 {
                        self.lecturas_search_active = true;
                        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                    } else {
                        if self.lecturas_search_active && hover != 53 {
                            self.lecturas_search_active = false;
                            if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                        }
                        if hover >= 100 {
                            let idx = (hover - 100) as usize;
                            let query = self.lecturas_search_query.to_lowercase();
                            let readings: Vec<_> = crate::db::get_all_readings().into_iter().filter(|(p, info)| {
                                if query.is_empty() { return true; }
                                let name = std::path::Path::new(p).file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
                                let author = info.author.to_lowercase();
                                name.contains(&query) || author.contains(&query)
                            }).collect();

                            let item_h = 140.0;
                            let gap = 20.0;
                            let height = self.window_size.height as f32;
                            let items_per_page = ((height - 400.0) / (item_h + gap)).floor().max(1.0) as usize;
                            let start_idx = self.lecturas_page_idx * items_per_page;
                            let abs_idx = start_idx + idx;

                            if abs_idx < readings.len() {
                                let (path, _) = &readings[abs_idx];
                                self.lecturas_search_active = false;
                                self.loading_timeout = Some(std::time::Instant::now());
                                self.loading_lectura_idx = Some(abs_idx);
                                if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                                
                                if let Ok(exe) = std::env::current_exe() {
                                    let mut cmd = std::process::Command::new(exe);
                                    cmd.arg(path);
                                    #[cfg(target_os = "windows")]
                                    {
                                        use std::os::windows::process::CommandExt;
                                        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                                    }
                                    cmd.spawn().ok();
                                }
                            }
                        }
                    }
                    return;
                }

                if self.page_input_active && hover != 23 {
                    self.page_input_active = false;
                    if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                }

                if hover == 30 {
                    self.top_menu_open = !self.top_menu_open;
                    if !self.top_menu_open {
                        self.recent_menu_open = false;
                    }
                    if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                } else if hover == 31 {
                    self.top_menu_open = false;
                    if let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                        
                        if let Ok(exe) = std::env::current_exe() {
                            self.loading_timeout = Some(std::time::Instant::now());
                            let mut cmd = std::process::Command::new(exe);
                            cmd.arg(path);
                            #[cfg(target_os = "windows")]
                            {
                                use std::os::windows::process::CommandExt;
                                cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                            }
                            cmd.spawn().ok();
                        }
                    }
                } else if hover == 32 {
                    self.recent_menu_open = true;
                    if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                } else if hover == 33 {
                    self.view_lecturas = true;
                    self.top_menu_open = false;
                    if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                } else if hover == 34 {
                    if !self.pdf_path.is_empty() && !self.pages.is_empty() {
                        let progress = crate::db::ReadingProgress {
                            title: self.pdf_title.clone(),
                            author: self.pdf_author.clone(),
                            current_page: self.get_current_page_idx() as u32,
                            total_pages: self.pages.len() as u32,
                        };
                        let _ = crate::db::save_progress(&self.pdf_path, &progress);
                    }
                    event_loop.exit();
                } else if hover == 35 {
                    self.recent_menu_open = false;
                    if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                } else if hover >= 36 {
                    let idx = (hover - 36) as usize;
                    let recents = load_recent_files();
                    if idx < recents.len() {
                        let path = &recents[idx];
                        self.top_menu_open = false;
                        self.recent_menu_open = false;
                        if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                        
                        if let Ok(exe) = std::env::current_exe() {
                            self.loading_timeout = Some(std::time::Instant::now());
                            let mut cmd = std::process::Command::new(exe);
                            cmd.arg(path);
                            #[cfg(target_os = "windows")]
                            {
                                use std::os::windows::process::CommandExt;
                                cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                            }
                            cmd.spawn().ok();
                        }
                    }
                } else if hover == 9 {
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
                        4 => self.calculate_fit_height_zoom(self.window_size.height),
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
            WindowEvent::MouseInput { state: winit::event::ElementState::Released, button: winit::event::MouseButton::Left, .. } => {
                if self.is_dragging_scrollbar {
                    self.is_dragging_scrollbar = false;
                    if let Some(window) = self.window.as_ref() { window.request_redraw(); }
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
                    } else if self.lecturas_search_active {
                        match &event.logical_key {
                            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Backspace) => {
                                self.lecturas_search_query.pop();
                                self.lecturas_page_idx = 0;
                                if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                                return;
                            }
                            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) | winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                                self.lecturas_search_active = false;
                                if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                                return;
                            }
                            winit::keyboard::Key::Character(c_str) => {
                                if c_str.len() == 1 {
                                    self.lecturas_search_query.push_str(c_str);
                                    self.lecturas_page_idx = 0;
                                }
                                if let Some(window) = self.window.as_ref() { window.request_redraw(); }
                                return;
                            }
                            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space) => {
                                self.lecturas_search_query.push(' ');
                                self.lecturas_page_idx = 0;
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
                                    let new_zoom = self.calculate_fit_height_zoom(self.window_size.height);
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
        
        if let Some(t) = self.loading_timeout {
            if t.elapsed().as_secs_f32() > 1.5 {
                self.loading_timeout = None;
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
        }

        let mut got_any = false;
        while let Ok(msg) = self.rx_worker.try_recv() {
            match msg {
                WorkerMessage::PageRendered { page_idx, zoom, scene } => {
                    println!("Received rendered page {}", page_idx);
                    if (zoom - self.zoom).abs() < 0.001 {
                        self.page_images.borrow_mut().insert(page_idx, CachedPage { scene, zoom });
                        self.record_access_and_evict(page_idx);
                        let zoom_key = (self.zoom * 1000.0) as u32;
                        self.requested_pages.borrow_mut().remove(&(page_idx, zoom_key));
                        got_any = true;
                    } else {
                        println!("Discarded page {} due to zoom mismatch", page_idx);
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
                if !self.page_images.borrow().is_empty() {
                    window.set_maximized(true);
                    window.set_visible(true);
                }
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
    pub pdf_title: String,
    pub pdf_author: String,
    pub pages: Vec<PageInfo>,
    pub logo_rgba: Option<Vec<u8>>,
    pub logo_width: u32,
    pub logo_height: u32,
    pub window_icon: Option<winit::window::Icon>,
    pub kill_pid: Option<u32>,
}

impl Gui {
    pub fn new(
        pdf_path: String,
        pdf_title: String,
        pdf_author: String,
        pages: Vec<PageInfo>,
        logo_rgba: Option<Vec<u8>>,
        logo_width: u32,
        logo_height: u32,
        window_icon: Option<winit::window::Icon>,
        kill_pid: Option<u32>,
    ) -> Self {
        Self {
            pdf_path,
            pdf_title,
            pdf_author,
            pages,
            logo_rgba,
            logo_width,
            logo_height,
            window_icon,
            kill_pid,
        }
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

        let logo_image = self.logo_rgba.as_ref().map(|rgba| {
            vello::peniko::ImageData {
                data: vello::peniko::Blob::new(std::sync::Arc::new(rgba.clone())),
                format: vello::peniko::ImageFormat::Rgba8,
                alpha_type: vello::peniko::ImageAlphaType::Alpha,
                width: self.logo_width,
                height: self.logo_height,
            }
        });
        
        let mut app = App {
            pages: self.pages,
            window: None,
            render_cx: vello::util::RenderContext::new(),
            render_surface: None,
            renderer: None,
            pdf_path: self.pdf_path,
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
            logo_image,
            logo_rgba: self.logo_rgba.clone(),
            logo_width: self.logo_width,
            logo_height: self.logo_height,
            window_icon: self.window_icon.clone(),
            layout_mode: LayoutMode::SinglePage,
            left_menu_open: false,
            top_menu_open: false,
            recent_menu_open: false,
            page_input_active: false,
            page_input_text: String::new(),
            lecturas_page_idx: 0,
            lecturas_search_query: String::new(),
            lecturas_search_active: false,
            surface_size: (1, 1),
            last_draw_time: std::time::Instant::now(),
            loading_timeout: None,
            loading_lectura_idx: None,
            kill_pid: self.kill_pid,
            pdf_title: self.pdf_title,
            pdf_author: self.pdf_author,
            view_lecturas: false,
            is_dragging_scrollbar: false,
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
    let page_paths_cache: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<(usize, &'static str), Option<kurbo::BezPath>>>> = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

    let font_keys = [
        "serif_regular", "serif_bold", "serif_italic", "serif_bold_italic",
        "sans_regular", "sans_bold", "sans_italic", "mono_regular", "default"
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
                let mut path_builder = kurbo::BezPath::new();
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
                                    path_builder.move_to((
                                        (char_x + start_p.x * scale_factor * h_squeeze) as f64,
                                        (char_y - start_p.y * scale_factor) as f64,
                                    ));
                                }

                                match curve {
                                    ab_glyph::OutlineCurve::Line(_, p2) => {
                                        path_builder.line_to((
                                            (char_x + p2.x * scale_factor * h_squeeze) as f64,
                                            (char_y - p2.y * scale_factor) as f64,
                                        ));
                                        last_point = Some(p2);
                                    }
                                    ab_glyph::OutlineCurve::Quad(_, p2, p3) => {
                                        path_builder.quad_to(
                                            ((char_x + p2.x * scale_factor * h_squeeze) as f64,
                                            (char_y - p2.y * scale_factor) as f64),
                                            ((char_x + p3.x * scale_factor * h_squeeze) as f64,
                                            (char_y - p3.y * scale_factor) as f64),
                                        );
                                        last_point = Some(p3);
                                    }
                                    ab_glyph::OutlineCurve::Cubic(_, p2, p3, p4) => {
                                        path_builder.curve_to(
                                            ((char_x + p2.x * scale_factor * h_squeeze) as f64,
                                            (char_y - p2.y * scale_factor) as f64),
                                            ((char_x + p3.x * scale_factor * h_squeeze) as f64,
                                            (char_y - p3.y * scale_factor) as f64),
                                            ((char_x + p4.x * scale_factor * h_squeeze) as f64,
                                            (char_y - p4.y * scale_factor) as f64),
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
                    Some(path_builder)
                } else {
                    None
                };
                let mut cache = page_paths_cache.lock().unwrap();
                cache.insert((page_idx, font_key), path_opt);
            }
        }

        // 3. Render page paths to target Scene
        let mut page_scene = vello::Scene::new();
        
        let rect = kurbo::Rect::new(0.0, 0.0, page_rect.width as f64, page_rect.height as f64);
        page_scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            1.0,
            kurbo::Affine::scale(zoom as f64),
            &rect,
        );
        page_scene.fill(
            vello::peniko::Fill::NonZero,
            kurbo::Affine::scale(zoom as f64),
            vello::peniko::Color::WHITE,
            None,
            &rect,
        );

        for &font_key in &font_keys {
            let path_opt = {
                let cache = page_paths_cache.lock().unwrap();
                cache.get(&(page_idx, font_key)).cloned().flatten()
            };
            if let Some(p) = path_opt {
                page_scene.fill(
                    vello::peniko::Fill::NonZero,
                    kurbo::Affine::scale(zoom as f64),
                    vello::peniko::Color::BLACK,
                    None,
                    &p,
                );
            }
        }

        for cmd in commands.iter() {
            if let DrawCommand::Image { image, transform, .. } = cmd {
                let mut decoded = None;
                if image.filter == "DCTDecode" {
                    decoded = image::load_from_memory(&image.data).ok();
                } else if image.filter == "FlateDecode" || image.filter.is_empty() {
                    use std::io::Read;
                    let raw_pixels = if image.filter == "FlateDecode" {
                        let mut decoder = flate2::read::ZlibDecoder::new(image.data.as_slice());
                        let mut buf = Vec::new();
                        let _ = decoder.read_to_end(&mut buf);
                        buf
                    } else {
                        image.data.clone()
                    };
                    
                    if !raw_pixels.is_empty() {
                        if image.color_space == "DeviceRGB" {
                            if let Some(img_buf) = image::RgbImage::from_raw(image.width, image.height, raw_pixels) {
                                decoded = Some(image::DynamicImage::ImageRgb8(img_buf));
                            }
                        } else if image.color_space == "DeviceGray" {
                            if let Some(img_buf) = image::GrayImage::from_raw(image.width, image.height, raw_pixels) {
                                decoded = Some(image::DynamicImage::ImageLuma8(img_buf));
                            }
                        } else {
                            eprintln!("Unsupported ColorSpace for FlateDecode: '{}'", image.color_space);
                        }
                    }
                } else {
                    eprintln!("Unsupported image Filter: '{}'", image.filter);
                }
                
                if let Some(img) = decoded {
                    let rgba8 = img.into_rgba8();
                    let raw_data = rgba8.into_raw();
                    let img_data = vello::peniko::ImageData {
                        data: vello::peniko::Blob::new(std::sync::Arc::new(raw_data)),
                        format: vello::peniko::ImageFormat::Rgba8,
                        alpha_type: vello::peniko::ImageAlphaType::Alpha,
                        width: image.width,
                        height: image.height,
                    };
                    let img_brush = vello::peniko::Brush::Image(vello::peniko::ImageBrush::new(img_data));
                    
                    let img_transform = kurbo::Affine::new([
                        transform[0] as f64 / image.width as f64,
                        -transform[1] as f64 / image.width as f64,
                        -transform[2] as f64 / image.height as f64,
                        transform[3] as f64 / image.height as f64,
                        transform[2] as f64 + transform[4] as f64,
                        page_rect.height as f64 - transform[3] as f64 - transform[5] as f64, 
                    ]);
                    page_scene.fill(
                        vello::peniko::Fill::NonZero,
                        kurbo::Affine::scale(zoom as f64) * img_transform,
                        &img_brush,
                        None,
                        &kurbo::Rect::new(0.0, 0.0, image.width as f64, image.height as f64),
                    );
                }
            }
        }
        page_scene.pop_layer();

        tx_response.send(WorkerMessage::PageRendered {
            page_idx,
            zoom,
            scene: page_scene,
        }).ok();
        proxy.send_event(()).ok();
        println!("Page {} rendered in {} ms", page_idx, start_time.elapsed().as_millis());
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

// Recent Files Utilities
fn get_recent_files_path() -> std::path::PathBuf {
    let mut path = if let Ok(profile) = std::env::var("USERPROFILE") {
        std::path::PathBuf::from(profile)
    } else if let Ok(home) = std::env::var("HOME") {
        std::path::PathBuf::from(home)
    } else {
        std::env::current_dir().unwrap_or_default()
    };
    path.push(".adobo_recent.txt");
    path
}

pub fn add_recent_file(file_path: &str) {
    if file_path.is_empty() { return; }
    let path = get_recent_files_path();
    let mut files = load_recent_files();
    
    files.retain(|x| x != file_path);
    files.insert(0, file_path.to_string());
    
    files.truncate(5);
    
    if let Ok(mut f) = std::fs::File::create(path) {
        use std::io::Write;
        for file in files {
            writeln!(f, "{}", file).ok();
        }
    }
}

pub fn load_recent_files() -> Vec<String> {
    let path = get_recent_files_path();
    let mut files = Vec::new();
    if let Ok(mut f) = std::fs::File::open(path) {
        use std::io::Read;
        let mut contents = String::new();
        if f.read_to_string(&mut contents).is_ok() {
            for line in contents.lines() {
                let line = line.trim();
                if !line.is_empty() {
                    if std::path::Path::new(line).exists() {
                        files.push(line.to_string());
                    }
                }
            }
        }
    }
    files
}

