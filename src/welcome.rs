use winit::application::ApplicationHandler;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId, Icon};
use winit::event::{WindowEvent, ElementState, MouseButton};
use std::sync::Arc;
use ab_glyph::{FontVec, Font, ScaleFont};
use tiny_skia::{Pixmap, Paint, Rect, Transform, Color, PixmapPaint};

pub struct WelcomeApp {
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    logo_pixmap: Option<Pixmap>,
    font: Option<FontVec>,
    menu_open: bool,
    mouse_pos: (f32, f32),
    is_hovering: bool,
    hover_progress: f32,
    start_time: std::time::Instant,
    last_draw_time: std::time::Instant,
}

impl ApplicationHandler for WelcomeApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window = match event_loop.create_window(
                Window::default_attributes()
                    .with_title("Adobo")
                    .with_inner_size(winit::dpi::LogicalSize::new(1000.0, 800.0))
                    .with_visible(false)
            ) {
                Ok(w) => Arc::new(w),
                Err(e) => {
                    eprintln!("Failed to create window: {}", e);
                    return;
                }
            };
            self.window = Some(window.clone());
            if let Ok(context) = softbuffer::Context::new(window.clone()) {
                if let Ok(surface) = softbuffer::Surface::new(&context, window.clone()) {
                    self.context = Some(context);
                    self.surface = Some(surface);
                    self.draw();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(_size) => {
                if let Some(w) = self.window.as_ref() { w.request_redraw(); }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = (position.x as f32, position.y as f32);
                let window = self.window.as_ref().unwrap();
                let size = window.inner_size();
                let mx = self.mouse_pos.0;
                let my = self.mouse_pos.1;
                let width = size.width as f32;
                let height = size.height as f32;

                let mut new_hover = false;
                let btn_w = 340.0;
                let btn_h = 70.0;
                let btn_x = (width - btn_w) / 2.0;
                let btn_y = height / 2.0 + 170.0;

                if self.start_time.elapsed().as_secs_f32() >= 1.2 {
                    if mx >= btn_x && mx <= btn_x + btn_w && my >= btn_y && my <= btn_y + btn_h {
                        new_hover = true;
                    }
                }

                if new_hover != self.is_hovering {
                    self.is_hovering = new_hover;
                    window.request_redraw();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if self.is_hovering {
                    if let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                        if let Ok(exe) = std::env::current_exe() {
                            std::process::Command::new(exe).arg(path).spawn().ok();
                            event_loop.exit();
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.draw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let elapsed = self.start_time.elapsed().as_secs_f32();
        
        let mut animating = false;
        if elapsed < 2.5 { animating = true; }
        if self.is_hovering && self.hover_progress < 1.0 { animating = true; }
        if !self.is_hovering && self.hover_progress > 0.0 { animating = true; }

        if animating {
            let next_draw = self.last_draw_time + std::time::Duration::from_millis(16);
            let now = std::time::Instant::now();
            if now >= next_draw {
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            } else {
                event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(next_draw));
            }
        } else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
    }
}

impl WelcomeApp {
    pub fn draw(&mut self) {
        if let Some(surface) = self.surface.as_mut() {
            let window = self.window.as_ref().unwrap();
            let size = window.inner_size();
            if size.width == 0 || size.height == 0 { return; }

            if surface.resize(std::num::NonZeroU32::new(size.width).unwrap(), std::num::NonZeroU32::new(size.height).unwrap()).is_err() {
                return;
            }

            let mut pixmap = Pixmap::new(size.width, size.height).unwrap();
            let width = size.width as f32;
            let height = size.height as f32;

            let elapsed = self.start_time.elapsed().as_secs_f32();
            let dt = self.last_draw_time.elapsed().as_secs_f32().min(0.1);
            self.last_draw_time = std::time::Instant::now();

            if self.is_hovering {
                self.hover_progress = (self.hover_progress + dt * 8.0).min(1.0);
            } else {
                self.hover_progress = (self.hover_progress - dt * 8.0).max(0.0);
            }

            let is_loading = elapsed < 1.0;
            
            let mut alpha = 0.0;
            if elapsed >= 1.2 {
                alpha = ((elapsed - 1.2) * 2.0).clamp(0.0, 1.0);
            }

            let mut bg_paint = Paint::default();
            if let Some(shader) = tiny_skia::LinearGradient::new(
                tiny_skia::Point::from_xy(0.0, 0.0),
                tiny_skia::Point::from_xy(width, height),
                vec![
                    tiny_skia::GradientStop::new(0.0, Color::from_rgba8(25, 25, 35, 255)),
                    tiny_skia::GradientStop::new(0.5, Color::from_rgba8(16, 16, 24, 255)),
                    tiny_skia::GradientStop::new(1.0, Color::from_rgba8(10, 10, 14, 255)),
                ],
                tiny_skia::SpreadMode::Pad,
                Transform::identity(),
            ) {
                bg_paint.shader = shader;
                pixmap.fill_rect(Rect::from_xywh(0.0, 0.0, width, height).unwrap(), &bg_paint, Transform::identity(), None);
            } else {
                pixmap.fill(Color::from_rgba8(15, 15, 20, 255));
            }

            // Logo Draw
            let ty = height / 2.0 - 50.0;
            let lw = 160.0;
            let lh = 160.0;
            let lx = (width - lw) / 2.0;
            
            let target_ly = ty - lh - 40.0;
            let start_ly = (height - lh) / 2.0;
            
            let mut ly = start_ly;
            if elapsed >= 1.0 {
                let t = ((elapsed - 1.0) * 1.5).clamp(0.0, 1.0);
                let ease_t = 1.0 - (1.0 - t).powi(3);
                ly = start_ly + (target_ly - start_ly) * ease_t;
            }

            if let Some(ref logo) = self.logo_pixmap {
                let sx = lw / logo.width() as f32;
                let sy = lh / logo.height() as f32;
                
                // Add shadow to logo
                let mut shadow_paint = Paint::default();
                shadow_paint.set_color(Color::from_rgba8(0, 0, 0, 100));
                shadow_paint.anti_alias = true;
                if let Some(_shadow_rect) = Rect::from_xywh(lx + 5.0, ly + 10.0, lw * 0.9, lh * 0.9) {
                     let mut pb = tiny_skia::PathBuilder::new();
                     let cx = lx + lw / 2.0;
                     let cy = ly + lh / 2.0 + 10.0;
                     let r = lw / 2.2;
                     pb.move_to(cx + r, cy);
                     let c = r * 0.55228;
                     pb.cubic_to(cx + r, cy + c, cx + c, cy + r, cx, cy + r);
                     pb.cubic_to(cx - c, cy + r, cx - r, cy + c, cx - r, cy);
                     pb.cubic_to(cx - r, cy - c, cx - c, cy - r, cx, cy - r);
                     pb.cubic_to(cx + c, cy - r, cx + r, cy - c, cx + r, cy);
                     pb.close();
                     if let Some(path) = pb.finish() {
                         pixmap.fill_path(&path, &shadow_paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
                     }
                }

                let transform = Transform::from_translate(lx, ly).pre_scale(sx, sy);
                pixmap.draw_pixmap(0, 0, logo.as_ref(), &PixmapPaint::default(), transform, None);
            }

            if is_loading {
                if let Some(ref f) = self.font {
                    let text = "Cargando...";
                    let size = 24.0;
                    let tw = measure_text(f, text, size);
                    let pulse = (elapsed * 6.0).sin() * 0.5 + 0.5;
                    let load_alpha = (100.0 + pulse * 155.0) as u8;
                    draw_text(&mut pixmap, f, text, (width - tw) / 2.0, ly + lh + 50.0, size, Color::from_rgba8(180, 180, 180, load_alpha));
                }
            } else {
                // Draw Welcome Text
                let title = "Bienvenido a Adobo";
                let title_size = 64.0;
                if let Some(ref f) = self.font {
                    let tw = measure_text(f, title, title_size);
                    let tx = (width - tw) / 2.0;
                    
                    // Text shadow
                    draw_text(&mut pixmap, f, title, tx + 2.0, ty + 3.0, title_size, Color::from_rgba8(0, 0, 0, (120.0 * alpha) as u8));
                    draw_text(&mut pixmap, f, title, tx, ty, title_size, Color::from_rgba8(255, 255, 255, (255.0 * alpha) as u8));

                    let p1 = "Una alternativa moderna y rápida para lectura de PDF.";
                    let p2 = "Diseñado en Rust para la mejor experiencia minimalista.";
                    let sub_size = 24.0;
                    let text_color = Color::from_rgba8(190, 190, 205, (255.0 * alpha) as u8);

                    let w1 = measure_text(f, p1, sub_size);
                    let w2 = measure_text(f, p2, sub_size);
                    let ty_p1 = ty + 75.0;
                    draw_text(&mut pixmap, f, p1, (width - w1) / 2.0, ty_p1, sub_size, text_color);
                    draw_text(&mut pixmap, f, p2, (width - w2) / 2.0, ty_p1 + 35.0, sub_size, text_color);
                    
                    // Draw Central Button
                    let btn_w = 340.0;
                    let btn_h = 70.0;
                    let btn_x = (width - btn_w) / 2.0;
                    let btn_y = height / 2.0 + 170.0;

                    let hover_val = self.hover_progress;
                    let btn_alpha = (255.0 * alpha) as u8;
                    
                    let c1 = [200.0, 60.0, 40.0];
                    let c2 = [245.0, 85.0, 55.0];
                    
                    let r = (c1[0] + (c2[0] - c1[0]) * hover_val) as u8;
                    let g = (c1[1] + (c2[1] - c1[1]) * hover_val) as u8;
                    let b = (c1[2] + (c2[2] - c1[2]) * hover_val) as u8;
                    
                    let mut paint = Paint::default();
                    paint.anti_alias = true;
                    if let Some(shader) = tiny_skia::LinearGradient::new(
                        tiny_skia::Point::from_xy(btn_x, btn_y),
                        tiny_skia::Point::from_xy(btn_x + btn_w, btn_y + btn_h),
                        vec![
                            tiny_skia::GradientStop::new(0.0, Color::from_rgba8(r, g, b, btn_alpha)),
                            tiny_skia::GradientStop::new(1.0, Color::from_rgba8(r.saturating_sub(30), g.saturating_sub(15), b.saturating_sub(10), btn_alpha)),
                        ],
                        tiny_skia::SpreadMode::Pad,
                        Transform::identity(),
                    ) {
                        paint.shader = shader;
                    } else {
                        paint.set_color(Color::from_rgba8(r, g, b, btn_alpha));
                    }
                    
                    // Button Shadow
                    if alpha > 0.0 {
                        let mut shadow_paint = Paint::default();
                        shadow_paint.anti_alias = true;
                        shadow_paint.set_color(Color::from_rgba8(0, 0, 0, (80.0 * alpha) as u8));
                        let shadow_offset = 6.0 + hover_val * 4.0;
                        let shadow_path = rounded_rect(btn_x, btn_y + shadow_offset, btn_w, btn_h, 16.0);
                        pixmap.fill_path(&shadow_path, &shadow_paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
                    }

                    // Button Background
                    let path = rounded_rect(btn_x, btn_y, btn_w, btn_h, 16.0);
                    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
                    
                    // Button border glow on hover
                    if hover_val > 0.0 && alpha > 0.0 {
                        let mut stroke = tiny_skia::Stroke::default();
                        stroke.width = 2.0;
                        let mut border_paint = Paint::default();
                        border_paint.anti_alias = true;
                        border_paint.set_color(Color::from_rgba8(255, 180, 150, (150.0 * hover_val * alpha) as u8));
                        pixmap.stroke_path(&path, &border_paint, &stroke, Transform::identity(), None);
                    }
                    
                    let btn_text = "Abrir Documento";
                    let b_tw = measure_text(f, btn_text, 26.0);
                    
                    let text_y_offset = -hover_val * 3.0;
                    draw_text(&mut pixmap, f, btn_text, btn_x + (btn_w - b_tw) / 2.0, btn_y + 45.0 + text_y_offset, 26.0, Color::from_rgba8(255, 255, 255, btn_alpha));
                }
            }

            if let Ok(mut buffer) = surface.buffer_mut() {
                for (index, pixel) in pixmap.pixels().iter().enumerate() {
                    let r = pixel.red();
                    let g = pixel.green();
                    let b = pixel.blue();
                    buffer[index] = (r as u32) << 16 | (g as u32) << 8 | (b as u32);
                }
                buffer.present().unwrap();
                self.window.as_ref().unwrap().set_visible(true);
            }
        }          }
            }
}

pub fn run_welcome_screen(
    logo_rgba: Option<Vec<u8>>,
    logo_w: u32,
    logo_h: u32,
    _window_icon: Option<Icon>,
) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;

    // Load Font
    let font_data = std::fs::read("C:\\Windows\\Fonts\\times.ttf").unwrap_or_else(|_| {
        std::fs::read("C:\\Windows\\Fonts\\arial.ttf").unwrap_or_default()
    });
    let font = FontVec::try_from_vec(font_data).ok();

    let logo_pixmap = if let Some(rgba) = logo_rgba {
        let mut p = Pixmap::new(logo_w, logo_h).unwrap();
        p.data_mut().copy_from_slice(&rgba);
        Some(p)
    } else {
        None
    };

    let mut app = WelcomeApp {
        window: None,
        surface: None,
        context: None,
        logo_pixmap,
        font,
        menu_open: false,
        mouse_pos: (0.0, 0.0),
        is_hovering: false,
        hover_progress: 0.0,
        start_time: std::time::Instant::now(),
        last_draw_time: std::time::Instant::now(),
    };

    event_loop.run_app(&mut app)?;

    Ok(())
}

fn measure_text(font: &FontVec, text: &str, size: f32) -> f32 {
    let font_ref = font.as_scaled(size);
    let mut width = 0.0;
    for c in text.chars() {
        let glyph_id = font.glyph_id(c);
        width += font_ref.h_advance(glyph_id);
    }
    width
}

fn draw_text(pixmap: &mut Pixmap, font: &FontVec, text: &str, start_x: f32, y: f32, size: f32, color: Color) {
    let font_ref = font.as_scaled(size);
    let mut current_x = start_x;
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;

    for c in text.chars() {
        let glyph_id = font.glyph_id(c);
        let advance = font_ref.h_advance(glyph_id);
        
        if let Some(outline) = font.outline(glyph_id) {
            let mut path_builder = tiny_skia::PathBuilder::new();
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
                let sx = current_x + start_p.x * (size / font.units_per_em().unwrap() as f32);
                let sy = y - start_p.y * (size / font.units_per_em().unwrap() as f32);

                if is_new_contour {
                    path_builder.move_to(sx, sy);
                }
                match curve {
                    ab_glyph::OutlineCurve::Line(_, p2) => {
                        let px = current_x + p2.x * (size / font.units_per_em().unwrap() as f32);
                        let py = y - p2.y * (size / font.units_per_em().unwrap() as f32);
                        path_builder.line_to(px, py);
                        last_point = Some(p2);
                    }
                    ab_glyph::OutlineCurve::Quad(_, p2, p3) => {
                        let px2 = current_x + p2.x * (size / font.units_per_em().unwrap() as f32);
                        let py2 = y - p2.y * (size / font.units_per_em().unwrap() as f32);
                        let px3 = current_x + p3.x * (size / font.units_per_em().unwrap() as f32);
                        let py3 = y - p3.y * (size / font.units_per_em().unwrap() as f32);
                        path_builder.quad_to(px2, py2, px3, py3);
                        last_point = Some(p3);
                    }
                    ab_glyph::OutlineCurve::Cubic(_, p2, p3, p4) => {
                        let px2 = current_x + p2.x * (size / font.units_per_em().unwrap() as f32);
                        let py2 = y - p2.y * (size / font.units_per_em().unwrap() as f32);
                        let px3 = current_x + p3.x * (size / font.units_per_em().unwrap() as f32);
                        let py3 = y - p3.y * (size / font.units_per_em().unwrap() as f32);
                        let px4 = current_x + p4.x * (size / font.units_per_em().unwrap() as f32);
                        let py4 = y - p4.y * (size / font.units_per_em().unwrap() as f32);
                        path_builder.cubic_to(px2, py2, px3, py3, px4, py4);
                        last_point = Some(p4);
                    }
                }
            }
            if let Some(path) = path_builder.finish() {
                pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
            }
        }
        
        current_x += advance;
    }
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> tiny_skia::Path {
    let mut pb = tiny_skia::PathBuilder::new();
    let c = radius * 0.4477;
    pb.move_to(x + radius, y);
    pb.line_to(x + w - radius, y);
    pb.cubic_to(x + w - c, y, x + w, y + c, x + w, y + radius);
    pb.line_to(x + w, y + h - radius);
    pb.cubic_to(x + w, y + h - c, x + w - c, y + h, x + w - radius, y + h);
    pb.line_to(x + radius, y + h);
    pb.cubic_to(x + c, y + h, x, y + h - c, x, y + h - radius);
    pb.line_to(x, y + radius);
    pb.cubic_to(x, y + c, x + c, y, x + radius, y);
    pb.close();
    pb.finish().unwrap()
}
