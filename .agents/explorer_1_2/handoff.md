# Handoff Report: Vello GPU Migration Analysis

## 1. Observation
We ran the compilation check command on the `ufreader` project:
```powershell
& "C:\Users\jreyn\.cargo\bin\cargo.exe" check
```
This resulted in 127 compilation errors. The relevant verbatim errors from the log file (`task-21.log`) include:

1. **Unresolved `wgpu` and softbuffer imports:**
   ```
   error[E0432]: unresolved import `wgpu`
    --> src\gui_vello.rs:8:5
     |
   8 | use wgpu::{InstanceDescriptor, Backends, PowerPreference, RequestAdapterOptions, DeviceDescriptor, Features, Limits};
     |     ^^^^
   ```
   ```
   error[E0433]: cannot find type `Context` in this scope
       --> src\gui_vello.rs:1220:33
        |
   1220 |             let context = match Context::new(window.clone()) {
        |                                 ^^^^^^^ use of undeclared type `Context`
   ```

2. **Missing `tiny-skia` types and functions:**
   ```
   error[E0425]: cannot find type `Pixmap` in this scope
     --> src\gui_vello.rs:95:25
      |
   95 |     logo_pixmap: Option<Pixmap>,
      |                         ^^^^^^ not found in this scope
   ```

3. **Legacy `buffer` references in presentation loop:**
   ```
   error[E0425]: cannot find value `buffer` in this scope
      --> src\gui_vello.rs:631:42
       |
   631 |             self.draw_splash_screen(&mut buffer, width, height);
       |                                          ^^^^^^ not found in this scope
   ```

4. **Missing Vello renderer methods:**
   ```
   error[E0599]: no method named `render_to_surface` found for mutable reference `&mut Renderer` in the current scope
      --> src\gui_vello.rs:506:18
       |
   506 |         renderer.render_to_surface(
       |         ---------^^^^^^^^^^^^^^^^^ method not found in `&mut Renderer`
   ```

5. **Worker Message field mismatch:**
   ```
   error[E0026]: variant `gui_vello::WorkerMessage::PageRendered` does not have a field named `pixmap`
       --> src\gui_vello.rs:1491:63
        |
   1491 |                 WorkerMessage::PageRendered { page_idx, zoom, pixmap } => {
        |                                                               ^^^^^^ variant `gui_vello::WorkerMessage::PageRendered` does not have this field
        |                                                               help: `gui_vello::WorkerMessage::PageRendered` has a field named `scene`
   ```

6. **Undefined `Path` type in worker path builder:**
   ```
   error[E0425]: cannot find type `Path` in this scope
       --> src\gui_vello.rs:1840:115
        |
   1840 | ...llections::HashMap<(usize, &'static str), Option<Path>>>> = ...
        |                                                     ^^^^ not found in this scope
   ```

---

## 2. Logic Chain
1. **Unresolved wgpu & softbuffer imports**: `wgpu` is not declared directly as a dependency in `Cargo.toml`. Since Vello re-exports it, we should use `vello::wgpu`. `softbuffer`'s `Context` and `Surface` are not imported in `gui_vello.rs`. However, since this is a GPU-rendering module, we must remove softbuffer completely rather than importing its types.
2. **Missing `tiny-skia` types**: The compiler cannot find `Pixmap`, `Paint`, `Transform`, `Rect`, `PathBuilder`, and `FillRule` because `tiny-skia` is not imported. Since we want a hardware-accelerated GUI thread, instead of importing `tiny-skia` to run software drawing, we should draw all overlays and UI elements (buttons, menus, text, splash screen) as vector shapes in the `vello::Scene`.
3. **Legacy `buffer` references**: The errors arise because a legacy softbuffer drawing loop (retained from copying `gui.rs`) attempts to write pixel data to a `buffer` variable that is no longer defined in `App::draw`. Removing this loop and only rendering the vector `vello::Scene` via WGPU fixes this.
4. **Vello renderer `render_to_surface` error**: In Vello 0.9.0, rendering is output to a `wgpu::TextureView` using `renderer.render_to_texture(...)`. The code was calling a non-existent `render_to_surface` method.
5. **Worker message & cache mismatch**: The worker thread in `gui_vello.rs` has already been correctly refactored to construct and send a `vello::Scene` in `WorkerMessage::PageRendered`. However, the receiving event loop (`about_to_wait()`) and cache `CachedPage` still expect `pixmap: Pixmap`. Aligning the receiver and cache to store `scene: Scene` resolves the mismatch.
6. **Worker path cache `Path` error**: The worker constructs glyph paths using `kurbo::BezPath`. The cache type is declared with `Path`, which is undefined. Changing the cache definition to `kurbo::BezPath` resolves the error.

---

## 3. Caveats
- We assumed Vello's re-exported `wgpu` version is fully compatible with Winit 0.30.
- We did not implement or compile the final refactored code (as this is a read-only investigation).
- We assumed the user's graphics driver fully supports Vulkan/DX12/Metal as required by `wgpu`.

---

## 4. Conclusion
The GPU migration of `gui_vello.rs` can be achieved by:
1. Cleaning up the legacy `softbuffer` and `tiny-skia` drawing code from `App::draw`.
2. Initializing WGPU surface/device/queue and `vello::Renderer` in `App::resumed`.
3. Reconfiguring the WGPU surface in `WindowEvent::Resized` to support window resizing.
4. Implementing a vector-based text helper (`draw_text`) and splash screen/overlay drawing routines that append vector paths directly onto the main `vello::Scene`.
5. Aligning `App`'s page caches and message receiver to store and display `vello::Scene` instances instead of software-rendered pixmaps.
6. Changing the worker thread's path cache to store `kurbo::BezPath`.

---

## 5. Verification Method
To independently verify the solution:
1. Apply the refactoring changes described in `analysis.md`.
2. Change `main.rs` to import and run `gui_vello::Gui` instead of `gui::Gui`.
3. Run the project check/build command:
   ```powershell
   & "C:\Users\jreyn\.cargo\bin\cargo.exe" check
   ```
   Verify that it compiles successfully without errors.
4. Run the application to verify the GPU-accelerated window launches, displays the vectorized splash screen, and loads PDF pages smoothly:
   ```powershell
   & "C:\Users\jreyn\.cargo\bin\cargo.exe" run -- test.pdf
   ```
