# Analysis of `gui_vello.rs` and Refactoring Strategy

This report contains the findings of the investigation into the compilation errors and software-rendering dependencies in `src/gui_vello.rs`, and outlines a detailed refactoring strategy to migrate the GUI entirely to `vello`/`wgpu`.

---

## 1. Compilation Errors Capture

Running `cargo check` yields 127 compilation errors in `src/gui_vello.rs`. The major categories of compilation errors are:

### A. Missing Imports & Undeclared Types (E0433 / E0425)
Many `tiny-skia` and `softbuffer` types are used but not imported in `gui_vello.rs`.
- **`Pixmap`**: E.g., line 95 (`logo_pixmap: Option<Pixmap>`), line 1704 (`logo_pixmap: Pixmap::load_png(...)`).
- **`Paint`**: E.g., line 868 (`Paint::default()`), line 870 (`Paint::default()`).
- **`Rect`**: E.g., line 873 (`Rect::from_xywh(...)`), line 876 (`Rect::from_xywh(...)`).
- **`Transform`**: E.g., line 874 (`Transform::identity()`), line 108 (`Transform::from_scale(...)`).
- **`PathBuilder` & `FillRule`**: E.g., line 968 (`PathBuilder::new()`), line 990 (`FillRule::Winding`).
- **`Context` & `Surface`**: E.g., line 1220 (`Context::new(...)`), line 1227 (`Surface::new(...)`).
- **`wgpu` module**: E.g., lines 69-71 (`wgpu::Surface`, `wgpu::Device`, `wgpu::Queue`) because `wgpu` is not declared as a direct dependency in `Cargo.toml`.

### B. Mismatched Channel Message Structs (E0026 / E0027 / E0560)
The background worker thread sends `WorkerMessage::PageRendered` with a `Scene` field, but the GUI thread (`App::about_to_wait`) expects a `pixmap` field.
- **Line 1491**: `WorkerMessage::PageRendered { page_idx, zoom, pixmap } => { ... }`
- **Line 1493**: `CachedPage { pixmap, zoom }`
- **Line 1704**: `logo_pixmap` refers to `Pixmap` instead of Vello's `peniko::Image` or similar.

### C. Mismatched Types in Software Blitting (E0308 / E0277)
The softbuffer pixel blitting code leftover from `gui.rs` mixes `usize` and `u32` (e.g., lines 1175, 1178, 1182) when attempting to copy `Pixmap` data to the screen buffer.
- **Line 1175**: `if dst_row >= height { break; }` (comparing `usize` to `u32`).
- **Line 1182**: `let dst_idx = dst_row * width + dst_col;` (multiplying `usize` by `u32`).

### D. Missing Methods
- **`draw_text` method not found**: Line 1163 (`self.draw_text(...)`) is missing from `App`'s implementation in `gui_vello.rs`.

---

## 2. Integration with Codebase

`src/gui_vello.rs` is designed to be a drop-in replacement for `src/gui.rs` using GPU-accelerated Vello rendering instead of CPU-based softbuffer + tiny-skia rendering.
- **`main.rs`**: Currently declares `pub mod gui_vello;` but runs the software-rendered GUI via `gui::Gui`. Once `gui_vello.rs` is fixed, `main.rs` can switch to using `gui_vello::Gui` directly.
- **`interpreter.rs`**: Defines the `DrawCommand` parsed from PDF files. The worker thread in `gui_vello.rs` maps these commands into `kurbo::BezPath` glyph outlines.
- **`parser.rs`**: Used by the background worker thread to parse metadata and content streams.

---

## 3. References to Software-Rendering Crates in `gui_vello.rs`

Below is a catalog of all software-rendering references to `tiny-skia` and `softbuffer` found in `src/gui_vello.rs`:

| Crate | Type / Function | Line Numbers | Purpose |
|---|---|---|---|
| `softbuffer` | `Context` | 1220, 1223, 1235 | CPU-based window surface context initialization |
| `softbuffer` | `Surface` | 1227, 1230, 1236, 1258 | CPU-based window surface allocation & resize |
| `softbuffer` | `buffer` / `buffer.present` | 520, 596, 631, 638, 644, 726, 920, 927, 1069, 1076, 1184, 1191, 1198 | Direct frame-buffer writing & presentation |
| `tiny-skia` | `Pixmap` | 95, 103, 105, 109, 111, 112, 866, 883, 905, 944, 961, 1054, 1098, 1169, 1704 | Render target for UI overlays, splash screen, and icon |
| `tiny-skia` | `Paint` | 868, 870, 884, 889, 900, 946, 948, 962, 988, 1001, 1013, 1040, 1047, 1100, 1102, 1115, 1142, 1147, 1157 | Style definitions for shapes and text |
| `tiny-skia` | `Rect` | 873, 876, 886, 951, 954, 964, 1020, 1023, 1107, 1109, 1117, 1123, 1126, 1148, 1158 | Layout boundaries for shapes |
| `tiny-skia` | `Transform` | 108, 113, 874, 877, 887, 891, 903, 952, 955, 965, 990, 1021, 1024, 1043, 1052, 1107, 1110, 1118, 1124, 1127, 1149, 1159 | Transformation matrices for drawing |
| `tiny-skia` | `PathBuilder` & `FillRule` | 968, 986, 990 | Path construction for arrow icons in pagination overlay |

---

## 4. Refactoring Strategy

To fix `gui_vello.rs`, we will replace all softbuffer and tiny-skia references with hardware-accelerated Vello vector rendering.

### A. Dependency Configuration
Since `wgpu` is not listed in `Cargo.toml`, we must use `vello::wgpu` for all WGPU types. This avoids adding a separate `wgpu` dependency and ensures strict version alignment with `vello` 0.9.0.

### B. Struct Fields Refactoring (`App`)
Modify `App` struct fields in `src/gui_vello.rs` as follows:
- Remove softbuffer's `context`.
- Update `surface` to `Option<vello::wgpu::Surface<'static>>`.
- Add `surface_format: Option<vello::wgpu::TextureFormat>` to cache the configured surface format.
- Update `device` to `Option<vello::wgpu::Device>`.
- Update `queue: Option<vello::wgpu::Queue>`.
- Replace `logo_pixmap: Option<Pixmap>` with `logo_image: Option<vello::peniko::Image>`.

```rust
pub struct App {
    window: Option<Rc<Window>>,
    surface: Option<vello::wgpu::Surface<'static>>,
    surface_format: Option<vello::wgpu::TextureFormat>,
    device: Option<vello::wgpu::Device>,
    queue: Option<vello::wgpu::Queue>,
    renderer: Option<vello::Renderer>,
    // ...
    logo_image: Option<vello::peniko::Image>,
    // ...
}
```

### C. WGPU & Vello Initialization (`ApplicationHandler::resumed`)
In `resumed`, request WGPU instance, surface, adapter, device, and queue using `pollster::block_on`, configure the surface, and construct `vello::Renderer`.

```rust
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

            // Initialize WGPU and Vello
            let instance = vello::wgpu::Instance::default();
            let surface = instance.create_surface(window.clone()).unwrap();
            let adapter = pollster::block_on(instance.request_adapter(&vello::wgpu::RequestAdapterOptions {
                power_preference: vello::wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })).expect("Failed to find wgpu adapter");
            
            let (device, queue) = pollster::block_on(adapter.request_device(
                &vello::wgpu::DeviceDescriptor {
                    label: None,
                    required_features: vello::wgpu::Features::empty(),
                    required_limits: vello::wgpu::Limits::default(),
                    memory_hints: Default::default(),
                },
                None,
            )).expect("Failed to create wgpu device");

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

            let renderer = vello::Renderer::new(
                &device,
                vello::RendererOptions {
                    surface_format: Some(format),
                    use_cpu: false,
                    antialiasing_support: vello::AaSupport::all(),
                    num_init_threads: None,
                },
            ).unwrap();

            self.window = Some(window);
            self.surface = Some(surface);
            self.surface_format = Some(format);
            self.device = Some(device);
            self.queue = Some(queue);
            self.renderer = Some(renderer);
            
            if size.width > 0 && size.height > 0 {
                self.zoom = self.calculate_fit_zoom(size.width, size.height);
                self.rendered_zoom = self.zoom;
                self.clear_cache();
                self.center_on_content(size.width, size.height);
                self.zoom_initialized = true;
            }
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }
```

### D. Window Resize Handler (`WindowEvent::Resized`)
When the window is resized, call `surface.configure` with the updated width and height.

```rust
            WindowEvent::Resized(size) => {
                self.window_size = size;
                if let (Some(surface), Some(device), Some(format)) = (self.surface.as_ref(), self.device.as_ref(), self.surface_format) {
                    if size.width > 0 && size.height > 0 {
                        let config = vello::wgpu::SurfaceConfiguration {
                            usage: vello::wgpu::TextureUsages::RENDER_ATTACHMENT,
                            format,
                            width: size.width,
                            height: size.height,
                            present_mode: vello::wgpu::PresentMode::Fifo,
                            alpha_mode: vello::wgpu::CompositeAlphaMode::Auto,
                            view_formats: vec![],
                            desired_maximum_frame_latency: 2,
                        };
                        surface.configure(device, &config);
                    }
                }
                // ...
```

### E. Text Drawing in Vello (`draw_text_to_scene`)
Implement a text rendering method on `App` that parses glyph outlines using `ab_glyph` and appends them to a `vello::Scene` using `kurbo::BezPath`.

```rust
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
```

### F. Page & Overlay Rendering (`App::draw`)
Replace the pixel blitting and softbuffer code with vector rendering.
- Render background: `scene.fill(..., vello::peniko::Color::from_rgb8(82, 86, 89), ...)` (gray background).
- Render page scenes: Draw cached `vello::Scene`s translated by the scroll/offsets, or white placeholder rectangles if not loaded.
- Render overlays: Use `scene.fill` with `kurbo::RoundedRect` and call `draw_text_to_scene` for labels.
- Render splash screen: Implement `draw_splash_screen_to_scene` using `vello::peniko::Brush::Image` for the logo.

```rust
    fn draw(&mut self, _window: &Window) {
        let width = self.window_size.width;
        let height = self.window_size.height;
        if width == 0 || height == 0 { return; }

        let device = self.device.as_ref().unwrap();
        let queue = self.queue.as_ref().unwrap();
        let surface = self.surface.as_ref().unwrap();
        let renderer = self.renderer.as_mut().unwrap();

        let surface_texture = match surface.get_current_texture() {
            Ok(t) => t,
            Err(_) => return,
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

        // 2. Draw Splash Screen if loading
        if page_count > 0 && is_loading {
            self.draw_splash_screen_to_scene(&mut scene, width as f64, height as f64);
            
            // Queue up initial page requests
            self.queue_page_requests(width, height);

            renderer.render_to_surface(
                device,
                queue,
                &scene,
                &surface_texture,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::from_rgb8(18, 18, 18),
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            ).unwrap();
            surface_texture.present();
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
        self.draw_ui_overlays_to_scene(&mut scene, width as f64, height as f64);

        // 5. Send pre-fetch requests
        self.queue_page_requests(width, height);

        // 6. Submit Render commands to GPU
        renderer.render_to_surface(
            device,
            queue,
            &scene,
            &surface_texture,
            &vello::RenderParams {
                base_color: vello::peniko::Color::from_rgb8(82, 86, 89),
                width,
                height,
                antialiasing_method: vello::AaConfig::Area,
            },
        ).unwrap();
        surface_texture.present();
    }
```

### G. Worker Thread Channel Handling (`App::about_to_wait`)
Fix type matching when receiving messages from the worker thread.

```rust
        while let Ok(msg) = self.rx_worker.try_recv() {
            match msg {
                WorkerMessage::PageRendered { page_idx, zoom, scene } => {
                    if (zoom - self.zoom).abs() < 0.001 {
                        self.page_images.borrow_mut().insert(page_idx, CachedPage { scene, zoom });
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
```

---

## 5. Summary of Recommended Actions
1. **Remove `softbuffer` completely** from `gui_vello.rs`.
2. **Replace `wgpu` imports** with `vello::wgpu`.
3. **Change logo type** to `Option<vello::peniko::Image>` and load it to GPU memory.
4. **Implement `draw_text_to_scene`** on `App`.
5. **Convert all manual blitting loops** in `App::draw` to Vello scene operations (rounded rects, image brushes, text paths).
6. **Update channel pattern matching** in `about_to_wait` to handle `scene` instead of `pixmap`.
7. **Switch `main.rs`** to invoke `gui_vello::Gui` instead of `gui::Gui`.
