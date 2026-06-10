# Refactoring Analysis: Migrating `gui_vello.rs` to Vello & WGPU

This report details the investigation of compilation errors in `src/gui_vello.rs` and provides a comprehensive, step-by-step refactoring strategy to complete the migration from the software-rendering pipeline (`softbuffer` + `tiny-skia`) to the hardware-accelerated pipeline (`vello` + `wgpu`).

---

## 1. Context and Codebase Integration
- **`src/main.rs`**: Currently imports `pub mod gui_vello;` but launches the software-rendered GUI defined in `src/gui.rs` via `gui::Gui::new`.
- **`src/gui_vello.rs`**: Intended to implement the hardware-accelerated rendering interface using `vello` and `wgpu`. It defines `pub struct Gui` (line 1589) and an internal `struct App` (line 67) that implements `winit::application::ApplicationHandler`.
- **Worker Thread**: `run_worker_thread` (line 1715) runs in the background. It reads PDF commands, outlines text using `ab_glyph`, builds vector paths (`kurbo::BezPath`), compiles them into `vello::Scene` instances, and sends them to the GUI thread via channel messages (`WorkerMessage::PageRendered`).

---

## 2. Compilation Errors Catalog (Baseline check)
The compilation of `gui_vello.rs` fails with **127 errors** due to:
1. **Unresolved `wgpu` Import**:
   - `wgpu` is not declared as a direct dependency in `Cargo.toml`. Since it is exported by `vello`, it must be imported as `vello::wgpu`.
2. **Missing `tiny-skia` and `softbuffer` types**:
   - Types like `Pixmap`, `Paint`, `Rect`, `Transform`, `PixmapPaint`, `PathBuilder`, and `FillRule` are not imported or declared, but are referenced extensively.
3. **Mismatched Drawing Pipelines**:
   - The `draw` method (lines 447–1201) contains a new Vello rendering section (lines 447–519) immediately followed by the old `softbuffer` drawing code (lines 520–1201).
   - This leads to errors where variables like `buffer` (the softbuffer pixel buffer) are referenced but do not exist in scope.
4. **Mismatched types in `WorkerMessage` and cache**:
   - `WorkerMessage::PageRendered` has been updated to use `scene: Scene` instead of `pixmap: Pixmap`, but the event-processing loop in `about_to_wait` still pattern-matches on `pixmap` and inserts it into `CachedPage`.
5. **No `render_to_surface` method**:
   - `vello::Renderer` in version 0.9.0 does not have a `render_to_surface` method. It must render using `render_to_texture` targeting a `wgpu::TextureView`.
6. **Missing helper methods**:
   - `draw_splash_screen` and `draw_text` are called but are not defined in `gui_vello.rs`.

---

## 3. Type Mapping Strategy

To completely remove `tiny-skia` and `softbuffer` from `gui_vello.rs`, map the software rendering types to their `vello`/`kurbo` equivalents as follows:

| Software Rendering Type | Vello / Kurbo Equivalent | Purpose |
|---|---|---|
| `softbuffer::Context` | `vello::wgpu::Instance` | WGPU entrypoint |
| `softbuffer::Surface` | `vello::wgpu::Surface<'static>` | Window presentation |
| `tiny_skia::Pixmap` | `vello::Scene` | Vector scene target |
| `tiny_skia::Paint` | `vello::peniko::Color` or `Brush` | Fill/stroke colors |
| `tiny_skia::Rect` | `kurbo::Rect` or `RoundedRect` | Rectangular bounds |
| `tiny_skia::Transform` | `kurbo::Affine` | Coordinates transformations |
| `tiny_skia::PathBuilder` | `kurbo::BezPath` | Custom vector paths (arrows/glyphs) |
| `tiny_skia::FillRule::Winding` | `vello::peniko::Fill::NonZero` | Fill style |

---

## 4. Refactoring Strategy

### Step 1: Import Cleanups
1. Remove all unused softbuffer and tiny-skia references.
2. Resolve `wgpu` imports by referencing the re-exported wgpu:
   ```rust
   use vello::wgpu;
   use vello::wgpu::{InstanceDescriptor, Backends, PowerPreference, RequestAdapterOptions, DeviceDescriptor, Features, Limits};
   use std::num::NonZeroU32; // for surface configuration
   ```

### Step 2: Fix `run_worker_thread` Cache & Pattern Mismatches
1. Update `page_paths_cache` declaration in `run_worker_thread` (line 1840) to store `kurbo::BezPath` instead of `Path`:
   ```rust
   let page_paths_cache: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<(usize, &'static str), Option<kurbo::BezPath>>>> =
       std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
   ```
2. Fix the worker message handler in `App::about_to_wait` (lines 1491–1493):
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

### Step 3: Implement WGPU & Vello Initialization in `resumed`
Replace the softbuffer creation in `fn resumed` (lines 1220–1236) with:
```rust
let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
    backends: wgpu::Backends::all(),
    ..Default::default()
});
let surface = unsafe { instance.create_surface(window.clone()) }.unwrap();

let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
    power_preference: wgpu::PowerPreference::HighPerformance,
    compatible_surface: Some(&surface),
    force_fallback_adapter: false,
})).expect("failed to find suitable adapter");

let (device, queue) = pollster::block_on(adapter.request_device(
    &wgpu::DeviceDescriptor {
        label: Some("Vello Device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: Default::default(),
    },
    None,
)).expect("failed to request device");

let surface_caps = surface.get_capabilities(&adapter);
let texture_format = surface_caps.formats.first().copied().unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
let config = wgpu::SurfaceConfiguration {
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
    format: texture_format,
    width: size.width,
    height: size.height,
    present_mode: wgpu::PresentMode::Fifo,
    alpha_mode: surface_caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Opaque),
    view_formats: vec![],
    desired_maximum_frame_latency: 2,
};
surface.configure(&device, &config);

let renderer = vello::Renderer::new(
    &device,
    vello::RendererOptions {
        use_cpu: false,
        antialiasing_support: vello::AaSupport::all(),
        num_init_threads: None,
    },
).unwrap();

self.window = Some(window.clone());
self.surface = Some(surface);
self.device = Some(device);
self.queue = Some(queue);
self.renderer = Some(renderer);
```

### Step 4: Handle Resizing in `window_event`
Replace `WindowEvent::Resized` handler (lines 1256–1262) with:
```rust
WindowEvent::Resized(size) => {
    self.window_size = size;
    if let (Some(device), Some(surface)) = (self.device.as_ref(), self.surface.as_ref()) {
        if size.width > 0 && size.height > 0 {
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: wgpu::TextureFormat::Bgra8Unorm, // or query capabilities format
                width: size.width,
                height: size.height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            surface.configure(device, &config);
        }
    }
    // ... rest of resized logic (fit zoom calculation)
}
```

### Step 5: Clean Up `draw` & Integrate Vello Rendering
1. Delete the entire old software rendering logic starting from line 520 to 1201.
2. In the `draw` method (lines 447–519), retrieve the texture view and render to it:
   ```rust
   let surface_texture = match surface.get_current_texture() {
       Ok(t) => t,
       Err(_) => return,
   };
   let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
   
   // ... build scene (append cached pages and draw UI overlays) ...

   renderer.render_to_texture(
       device,
       queue,
       &scene,
       &view,
       &vello::RenderParams {
           base_color: vello::peniko::Color::from_rgb8(18, 18, 18),
           width,
           height,
           antialiasing_method: vello::AaConfig::Area,
       },
   ).unwrap();
   surface_texture.present();
   ```

### Step 6: Vector UI Overlay Rendering
1. **Text Rendering**: Implement `draw_text_vello` on `App` that draws text using font glyphs directly into the `vello::Scene`:
   ```rust
   fn draw_text_vello(
       &self,
       scene: &mut Scene,
       text: &str,
       start_x: f32,
       y: f32,
       size: f32,
       font: &FontVec,
       color: vello::peniko::Color,
   ) {
       let scale_factor = size / font.units_per_em().unwrap_or(1000.0);
       let mut current_x = start_x;
       let mut path_builder = kurbo::BezPath::new();
       
       for c in text.chars() {
           let glyph_id = font.glyph_id(c);
           let actual_w = font.h_advance_unscaled(glyph_id) * scale_factor;
           
           if let Some(outline) = font.outline(glyph_id) {
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
           }
           current_x += actual_w;
       }
       
       scene.fill(
           vello::peniko::Fill::NonZero,
           kurbo::Affine::IDENTITY,
           color,
           None,
           &path_builder,
       );
   }
   ```
2. **UI Rectangles & Borders**: Replace tiny-skia `Rect` fills with `scene.fill` calls:
   ```rust
   let rect = kurbo::RoundedRect::new(x, y, x + w, y + h, radius);
   scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, fill_color, None, &rect);
   scene.stroke(&kurbo::Stroke::new(border_width), kurbo::Affine::IDENTITY, border_color, None, &rect);
   ```
3. **Icons & Custom Paths**: Rewrite the navigation arrow custom path building to use `kurbo::BezPath` and append it directly to `scene` instead of tiny-skia `PathBuilder` and `Pixmap`.
