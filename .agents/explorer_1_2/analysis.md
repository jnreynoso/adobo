# Analysis Report: `ufreader` GPU Migration and Vello Refactoring

## Executive Summary
This report analyzes the compilation errors and codebase integration of `src/gui_vello.rs` in `ufreader`. The current implementation contains a hybrid of newer vector GPU-rendering logic (Vello/WGPU) and legacy software-rendering logic (tiny-skia/softbuffer), resulting in 127 compilation errors. We outline the exact compilation issues, identify all software-rendering references, and present a detailed refactoring strategy to complete the GPU migration.

---

## 1. Compilation Error Analysis
Running `cargo check` reveals 127 compilation errors in `gui_vello.rs`. They fall into three categories:

### A. Missing Imports & Unresolved Modules
- **`wgpu` Modules**: `wgpu` is not directly declared in `Cargo.toml`. Imports like `use wgpu::{...}` at the top of `gui_vello.rs` (line 8) are unresolved. Cargo recommends using `vello::wgpu`.
- **Softbuffer Types**: `Context` and `Surface` are used in `resumed()` (lines 1220, 1227) but are not imported.
- **Tiny-Skia Types**: Types like `Pixmap`, `Paint`, `Transform`, `PixmapPaint`, `Rect`, `PathBuilder`, and `FillRule` are used for overlays (lines 850–1200) but are completely undefined in the module.
- **Standard Library Types**: `NonZeroU32` is not imported (line 1259), and `Path` is referenced (line 1840) but is undefined (conflicting with `std::path::Path` vs `tiny_skia::Path`).

### B. Type & Field Mismatches in Thread Integration
- **Worker Message & Cache**: `WorkerMessage::PageRendered` has a field `scene: Scene` (line 51), but `about_to_wait()` tries to match on `pixmap` (line 1491):
  ```rust
  WorkerMessage::PageRendered { page_idx, zoom, pixmap } => { ... } // Expected 'scene'
  ```
  `CachedPage` expects `scene: Scene` (line 63) but the code tries to instantiate it with `pixmap` (line 1493).
- **Worker Paths Cache**: `page_paths_cache` is defined with type `Option<Path>` (line 1840). Since `Path` is unresolved and `run_worker_thread` builds paths using `kurbo::BezPath` (line 1967), this should be `Option<kurbo::BezPath>`.

### C. Legacy Draw Pipeline Leftovers
- **Double Presentation Loop**: `App::draw` (lines 447–1202) contains both a new Vello rendering pipeline (lines 447–518) and a legacy softbuffer rendering pipeline (lines 520–1202). The legacy code tries to access a non-existent `buffer` variable, leading to dozens of `cannot find value buffer in this scope` errors.
- **Missing App Helpers**: `App` is missing definitions for `draw_splash_screen` and `draw_text` (which were software-rendering helper methods in `gui.rs`).

---

## 2. Codebase Integration
Currently, `src/main.rs` launches `gui::Gui`. `gui_vello::Gui` has an identical public interface:

| Struct | Module | Entrypoint Method | Parameter |
|---|---|---|---|
| `gui::Gui` | `src/gui.rs` | `run(self) -> Result<(), Box<dyn Error>>` | `pages: Vec<gui::PageInfo>` |
| `gui_vello::Gui` | `src/gui_vello.rs` | `run(self) -> Result<(), Box<dyn Error>>` | `pages: Vec<gui_vello::PageInfo>` |

### Integration Strategy:
1. In `src/main.rs`, replace `use gui::Gui;` and `gui::PageInfo` with:
   ```rust
   use gui_vello::Gui;
   ```
2. Replace `gui::PageInfo` references in `main.rs` with `gui_vello::PageInfo`.
3. In `gui_vello.rs`, clean up the `Gui::run` implementation to invoke winit's `EventLoop` with the refactored `App`.

---

## 3. Software-Rendering References in `gui_vello.rs`
The following table lists every reference to software-rendering crates (`tiny-skia`, `softbuffer`) in `gui_vello.rs` that must be refactored or removed:

| Line Number(s) | Symbol / Variable | Crate Origin | Current Purpose | Refactoring Action |
|---|---|---|---|---|
| `70, 71, 1220, 1227` | `Context`, `Surface` | `softbuffer` | Software window presentation | **Remove completely**. Replace with WGPU Surface and SwapChain configuration. |
| `95, 1704` | `logo_pixmap` | `tiny-skia` | Decoding and holding `logo.png` | **Refactor** to `Option<vello::peniko::Image>`, using `tiny-skia` only for initial PNG decoding. |
| `102–129` | `load_window_icon()` | `tiny-skia` | Setting window icon | **Keep** as helper, using `tiny-skia` only for window-system icon generation. |
| `631, 850–1200` | `buffer` | `softbuffer` | Rendering buffer for display | **Remove completely**. All outputs go through `vello::Scene` -> `wgpu::Surface`. |
| `866, 944, 1098` | `Pixmap::new(...)` | `tiny-skia` | Software render target for overlay UI | **Remove completely**. Render UI directly onto `vello::Scene` using vector primitives. |
| `868–871, 884` | `Paint` | `tiny-skia` | Fill styles for software rendering | **Replace** with `vello::peniko::Color` or `vello::peniko::Brush`. |
| `873, 876` | `Rect::from_xywh` | `tiny-skia` | Rectangle primitives | **Replace** with `kurbo::Rect` or `kurbo::RoundedRect`. |
| `968` | `PathBuilder` | `tiny-skia` | Path geometry construction | **Replace** with `kurbo::BezPath`. |
| `990` | `FillRule` | `tiny-skia` | Path winding rule | **Replace** with `vello::peniko::Fill::NonZero` or `Fill::EvenOdd`. |

---

## 4. Refactoring Strategy

### R1. WGPU / Vello GUI Thread Integration
We need to replace the softbuffer-based initialization in `App::resumed` with a synchronous WGPU and Vello initialization. Winit's event loop executes synchronously, so we must block on the async adapter and device retrieval using the `pollster` crate:

```rust
// In App::resumed()
let size = window.inner_size();
self.window_size = size;

// 1. Initialize WGPU Instance
let instance = vello::wgpu::Instance::default();

// 2. Create Window Surface
let surface = instance.create_surface(window.clone()).unwrap();

// 3. Request Adapter (synchronously via pollster)
let adapter = pollster::block_on(instance.request_adapter(&vello::wgpu::RequestAdapterOptions {
    power_preference: vello::wgpu::PowerPreference::HighPerformance,
    compatible_surface: Some(&surface),
    force_fallback_adapter: false,
})).unwrap();

// 4. Request Device & Queue (synchronously via pollster)
let (device, queue) = pollster::block_on(adapter.request_device(
    &vello::wgpu::DeviceDescriptor {
        label: None,
        required_features: vello::wgpu::Features::empty(),
        required_limits: vello::wgpu::Limits::default(),
        memory_hints: Default::default(),
    },
    None,
)).unwrap();

// 5. Configure SwapChain/Surface
let capabilities = surface.get_capabilities(&adapter);
let format = capabilities.formats[0];
let config = vello::wgpu::SurfaceConfiguration {
    usage: vello::wgpu::TextureUsages::RENDER_ATTACHMENT,
    format,
    width: size.width,
    height: size.height,
    present_mode: vello::wgpu::PresentMode::Fifo,
    alpha_mode: capabilities.alpha_modes[0],
    view_formats: vec![],
    desired_maximum_frame_latency: 2,
};
surface.configure(&device, &config);

// 6. Create Vello Renderer
let renderer = vello::Renderer::new(
    &device,
    vello::RendererOptions {
        surface_format: Some(format),
        use_cpu: false,
        antialiasing_support: vello::AaSupport::all(),
        num_init_threads: None,
    },
).unwrap();

self.window = Some(window.clone());
self.surface = Some(surface);
self.surface_config = Some(config); // Add this field to App to support Resizing
self.device = Some(device);
self.queue = Some(queue);
self.renderer = Some(renderer);
```

### R2. Window Resizing
In `WindowEvent::Resized(size)`, reconfigure the surface instead of resizing softbuffer:
```rust
WindowEvent::Resized(size) => {
    self.window_size = size;
    if let (Some(surface), Some(device), Some(config)) = (self.surface.as_mut(), self.device.as_ref(), self.surface_config.as_mut()) {
        if size.width > 0 && size.height > 0 {
            config.width = size.width;
            config.height = size.height;
            surface.configure(device, config);
        }
    }
    if let Some(window) = self.window.as_ref() {
        window.request_redraw();
    }
}
```

### R3. Vectorizing the UI Drawing & Splash Screen
Instead of drawing overlays to `Pixmap` and software-blending them into `buffer`, we should draw them as vector geometry in `vello::Scene` during `App::draw()`.

#### Vector Text Drawing Helper
Add a helper method `draw_text` to `App` using `ab_glyph` to output paths directly into `vello::Scene`:
```rust
fn draw_text(scene: &mut Scene, font: &FontVec, text: &str, x: f32, y: f32, size: f32, color: peniko::Color) {
    let scale_factor = size / font.units_per_em().unwrap_or(1000.0);
    let mut current_x = x;
    for c in text.chars() {
        let glyph_id = font.glyph_id(c);
        let actual_w = font.h_advance_unscaled(glyph_id) * scale_factor;
        if let Some(outline) = font.outline(glyph_id) {
            let mut path_builder = kurbo::BezPath::new();
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
                        (current_x + start_p.x * scale_factor) as f64,
                        (y - start_p.y * scale_factor) as f64,
                    ));
                }
                match curve {
                    ab_glyph::OutlineCurve::Line(_, p2) => {
                        path_builder.line_to((
                            (current_x + p2.x * scale_factor) as f64,
                            (y - p2.y * scale_factor) as f64,
                        ));
                        last_point = Some(p2);
                    }
                    ab_glyph::OutlineCurve::Quad(_, p2, p3) => {
                        path_builder.quad_to(
                            ((current_x + p2.x * scale_factor) as f64, (y - p2.y * scale_factor) as f64),
                            ((current_x + p3.x * scale_factor) as f64, (y - p3.y * scale_factor) as f64),
                        );
                        last_point = Some(p3);
                    }
                    ab_glyph::OutlineCurve::Cubic(_, p2, p3, p4) => {
                        path_builder.curve_to(
                            ((current_x + p2.x * scale_factor) as f64, (y - p2.y * scale_factor) as f64),
                            ((current_x + p3.x * scale_factor) as f64, (y - p3.y * scale_factor) as f64),
                            ((current_x + p4.x * scale_factor) as f64, (y - p4.y * scale_factor) as f64),
                        );
                        last_point = Some(p4);
                    }
                }
            }
            scene.fill(
                peniko::Fill::NonZero,
                kurbo::Affine::IDENTITY,
                color,
                None,
                &path_builder,
            );
        }
        current_x += actual_w;
    }
}
```

#### Splash Screen Refactoring
```rust
fn draw_splash_screen(&self, scene: &mut Scene, width: f32, height: f32) {
    let font = &self.default_font;
    let title = "UfReader";
    let title_size = 36.0f32;
    let text_color = peniko::Color::WHITE;

    let splash_w = 400.0f32;
    let splash_h = 400.0f32;
    let lx = (width - splash_w) / 2.0;
    let ly = (height - splash_h) / 2.0;

    // Draw splash background card
    let card_rect = kurbo::RoundedRect::new(lx as f64, ly as f64, (lx + splash_w) as f64, (ly + splash_h) as f64, 10.0);
    scene.fill(
        peniko::Fill::NonZero,
        kurbo::Affine::IDENTITY,
        peniko::Color::from_rgb8(25, 25, 25),
        None,
        &card_rect,
    );

    // Draw logo image if present
    if let Some(ref logo) = self.logo_pixmap {
        // Draw image directly onto Vello scene
        let img_w = 160.0;
        let img_h = 160.0;
        let img_x = lx + (splash_w - img_w) / 2.0;
        let img_y = ly + 40.0;
        
        let sx = img_w as f64 / logo.width as f64;
        let sy = img_h as f64 / logo.height as f64;
        let transform = kurbo::Affine::translate((img_x as f64, img_y as f64)) * kurbo::Affine::scale_non_uniform(sx, sy);
        
        scene.draw_image(logo, transform);
    }

    let text_y = ly + 260.0f32;
    let tw = self.measure_text_width(title, title_size, font);
    let tx = lx + (splash_w - tw) / 2.0;
    Self::draw_text(scene, font, title, tx, text_y, title_size, text_color);

    let sub = "Cargando documento...";
    let sub_size = 16.0f32;
    let sub_color = peniko::Color::from_rgb8(150, 150, 150);
    let sw = self.measure_text_width(sub, sub_size, font);
    let sx = lx + (splash_w - sw) / 2.0;
    Self::draw_text(scene, font, sub, sx, text_y + 40.0, sub_size, sub_color);
}
```

#### Vectorizing the Navigation and Zoom Overlays
For each overlay (menu, zoom buttons, pagination), draw them in `App::draw` by appending shapes directly to the `Scene`.
Example zoom buttons drawing:
```rust
let overlay_rect = kurbo::RoundedRect::new(overlay_x as f64, overlay_y as f64, (overlay_x + overlay_width) as f64, (overlay_y + overlay_height) as f64, 8.0);
scene.fill(
    peniko::Fill::NonZero,
    kurbo::Affine::IDENTITY,
    peniko::Color::from_rgba8(25, 25, 25, 220),
    None,
    &overlay_rect,
);

// Draw buttons inside the overlay
let draw_btn = |scene: &mut Scene, x: f32, label: &str, hovered: bool| {
    let btn_x = overlay_x + x;
    let btn_y = overlay_y + 12.0;
    let btn_size = 76.0;
    let rect = kurbo::RoundedRect::new(btn_x as f64, btn_y as f64, (btn_x + btn_size) as f64, (btn_y + btn_size) as f64, 4.0);
    let color = if hovered { peniko::Color::from_rgb8(70, 70, 70) } else { peniko::Color::from_rgb8(40, 40, 40) };
    
    scene.fill(peniko::Fill::NonZero, kurbo::Affine::IDENTITY, color, None, &rect);
    
    let tw = self.measure_text_width(label, 36.0, font);
    Self::draw_text(scene, font, label, btn_x + (btn_size - tw) / 2.0, btn_y + 52.0, 36.0, peniko::Color::WHITE);
};

draw_btn(&mut scene, 20.0, "-", hover_state == 2);
draw_btn(&mut scene, 306.0, "+", hover_state == 3);
draw_btn(&mut scene, 408.0, "R", hover_state == 4);
```

### R4. Presentation Pipeline Fixes
Replace the entire end of `App::draw` (lines 506 onwards) with:
```rust
        // Render scene to surface texture
        let surface_texture_view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        renderer.render_to_texture(
            device,
            queue,
            &scene,
            &surface_texture_view,
            &vello::RenderParams {
                base_color: vello::peniko::Color::from_rgb8(18, 18, 18),
                width,
                height,
                antialiasing_method: vello::AaConfig::Area,
            },
        ).unwrap();
        surface_texture.present();
```
This entirely bypasses softbuffer, outputting vector calculations directly onto the GPU swapchain.

---

## 5. Worker Thread Refactoring
In `run_worker_thread`:
1. Change the path cache type to:
   ```rust
   let page_paths_cache: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<(usize, &'static str), Option<kurbo::BezPath>>>> = ...
   ```
2. In `about_to_wait()` (line 1491) of `App` event loop:
   ```rust
   WorkerMessage::PageRendered { page_idx, zoom, scene } => {
       if (zoom - self.zoom).abs() < 0.001 {
           self.page_images.borrow_mut().insert(page_idx, CachedPage { scene, zoom });
           self.record_access_and_evict(page_idx);
           let zoom_key = (self.zoom * 1000.0) as u32;
           self.requested_pages.borrow_mut().remove(&(page_idx, zoom_key));
           got_any = true;
       }
   }
   ```
This aligns the worker communication channel with the vector pipeline.
