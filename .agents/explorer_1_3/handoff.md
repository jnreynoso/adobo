# Handoff Report

## 1. Observation
Compilation of `ufreader` with `gui_vello.rs` enabled fails with 127 compiler errors. Specific observed errors and references include:
- **Unresolved `wgpu` import** at `src\gui_vello.rs:8:5`:
  ```
  error[E0432]: unresolved import `wgpu`
    --> src\gui_vello.rs:8:5
  8 | use wgpu::{InstanceDescriptor, Backends, PowerPreference, RequestAdapterOptions, DeviceDescriptor, Features, Limits};
    |     ^^^^
  ```
- **Undeclared `Pixmap` type** at `src\gui_vello.rs:95:25`:
  ```
  error[E0425]: cannot find type `Pixmap` in this scope
    --> src\gui_vello.rs:95:25
  95 |     logo_pixmap: Option<Pixmap>,
     |                         ^^^^^^ not found in this scope
  ```
- **Undeclared softbuffer variable `buffer`** at `src\gui_vello.rs:631:42`:
  ```
  error[E0425]: cannot find value `buffer` in this scope
     --> src\gui_vello.rs:631:42
  631 |             self.draw_splash_screen(&mut buffer, width, height);
      |                                          ^^^^^^ not found in this scope
  ```
- **Missing field `context`** on `App` at `src\gui_vello.rs:1235:18`:
  ```
  error[E0609]: no field `context` on type `&mut gui_vello::App`
      --> src\gui_vello.rs:1235:18
  1235 |             self.context = Some(context);
       |                  ^^^^^^^ unknown field
  ```
- **Mismatched pattern matching field `pixmap`** at `src\gui_vello.rs:1491:63`:
  ```
  error[E0026]: variant `gui_vello::WorkerMessage::PageRendered` does not have a field named `pixmap`
      --> src\gui_vello.rs:1491:63
  1491 |                 WorkerMessage::PageRendered { page_idx, zoom, pixmap } => {
       |                                                               ^^^^^^ variant `gui_vello::WorkerMessage::PageRendered` does not have this field
  ```
- **Missing `render_to_surface` method** on `vello::Renderer` at `src\gui_vello.rs:506:18`:
  ```
  error[E0599]: no method named `render_to_surface` found for mutable reference `&mut Renderer` in the current scope
     --> src\gui_vello.rs:506:18
  506 |         renderer.render_to_surface(
      |         ---------^^^^^^^^^^^^^^^^^ method not found in `&mut Renderer`
  ```

---

## 2. Logic Chain
1. **Unresolved `wgpu`**: Because the `wgpu` dependency is not declared in `Cargo.toml` but is a dependency of `vello`, it must be imported through the re-export `vello::wgpu`.
2. **Missing `Pixmap` and other softbuffer/tiny-skia types**: The compiler errors reveal that the codebase was copied from the software-rendered version (`gui.rs`) and retains code that references `Pixmap`, `Paint`, `Rect`, `Transform`, etc. from `tiny-skia` and `softbuffer`.
3. **Mismatched Drawing Method**: The `draw` method contains a Vello rendering section followed immediately by the old softbuffer drawing/blitting loop, which assumes variables like `buffer` are present. By deleting the old softbuffer blitting code and implementing overlay drawing directly in Vello via `Scene` additions, these errors will be completely resolved.
4. **Resizing wgpu surface**: The `WindowEvent::Resized` logic currently calls `surface.resize(w, h)` which is a softbuffer API. For wgpu, `surface.configure(&device, &config)` must be called instead.
5. **Worker thread output**: The background thread `run_worker_thread` is structured to produce a vector-based `vello::Scene` using `kurbo::BezPath` outlines rather than a pixel-based `Pixmap`. The event loop thread's channel receiver must be updated to pattern-match `{ page_idx, zoom, scene }` and store it in the page cache instead of `{ page_idx, zoom, pixmap }`.

---

## 3. Caveats
- Since this is a read-only investigation, no code modifications were applied.
- The window icon loading (`load_window_icon`) currently uses `tiny-skia` to resize `logo.png`. If `tiny-skia` is to be completely removed from the project, a replacement PNG decoder (like `image` or `png`) will need to be added to `Cargo.toml`, or the icon loading logic must be removed.

---

## 4. Conclusion
The compilation errors in `gui_vello.rs` are caused by an incomplete port from `gui.rs` where the initialization methods, winit events, and UI overlays are still referencing the old softbuffer/tiny-skia software-rendering architecture. 
To complete the port, the old softbuffer code in the `draw` method must be removed, the `App` initialization in `resumed` updated to set up WGPU and Vello, and the text/rect overlay drawings refactored to add vector shapes directly to the `vello::Scene`.

---

## 5. Verification Method
To verify the refactoring plan:
1. Apply the changes detailed in `analysis.md` to `src/gui_vello.rs` and update `main.rs` to run `gui_vello::Gui::new` instead of `gui::Gui::new`.
2. Compile and check the codebase using:
   ```powershell
   C:\Users\jreyn\.cargo\bin\cargo.exe check --bin ufreader
   ```
3. Run the application to verify that the GUI displays successfully using hardware acceleration:
   ```powershell
   C:\Users\jreyn\.cargo\bin\cargo.exe run -- test.pdf
   ```
