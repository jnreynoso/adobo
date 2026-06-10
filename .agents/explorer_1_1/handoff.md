# Handoff Report — Vello Refactoring Analysis

## 1. Observation
- Run command output for `cargo check`:
  ```
  error[E0433]: cannot find type `Pixmap` in this scope
      --> src\gui_vello.rs:1704:26
  
  error[E0433]: cannot find module or crate `wgpu` in this scope
    --> src\gui_vello.rs:69:21
  
  error[E0609]: no field `context` on type `&mut gui_vello::App`
      --> src\gui_vello.rs:1235:18
  
  error[E0026]: variant `gui_vello::WorkerMessage::PageRendered` does not have a field named `pixmap`
      --> src\gui_vello.rs:1491:63
  
  error[E0599]: no method named `draw_text` found for mutable reference `&mut gui_vello::App` in the current scope
      --> src\gui_vello.rs:1163:30
  ```
- File `src/gui_vello.rs` has no imports for `softbuffer` or `tiny_skia`, but uses `Context` (line 1220), `Surface` (line 1227), `Pixmap` (line 866), `Paint` (line 868), and `Rect` (line 873) in `App::draw` and `resumed`.
- `Cargo.toml` has `vello = "0.9.0"`, `kurbo = "0.13.1"`, `softbuffer = "0.4.8"`, `tiny-skia = "0.12.0"`, but does NOT list `wgpu`.
- The background worker thread function `run_worker_thread` in `src/gui_vello.rs` is fully refactored to compile character glyphs from `ab_glyph` into `kurbo::BezPath` and build/send a `vello::Scene` across a channel using `WorkerMessage::PageRendered { ..., scene }` (lines 2063-2096).
- `main.rs` is currently hardcoded to import and run the software-rendered `gui::Gui` instead of `gui_vello::Gui` (lines 3, 9, 68).

## 2. Logic Chain
- Since `cargo check` fails with 127 compilation errors due to missing type definitions (e.g. `Pixmap`, `Paint`, `wgpu`), we know `gui_vello.rs` contains active references to software-rendering crates (`tiny-skia` and `softbuffer`) that it does not import.
- Because the background worker thread is already successfully constructing and sending `vello::Scene`s, the background architecture is correct, but the frontend GUI thread is broken because it expects a `Pixmap` in message handling (line 1491) and tries to blit pixels to a softbuffer CPU frame buffer (line 708 onwards).
- Since `wgpu` is not declared in `Cargo.toml` but Vello 0.9.0 re-exports it, we can resolve the missing `wgpu` errors by importing it from `vello::wgpu`.
- By removing `softbuffer` and `tiny-skia` references from `gui_vello.rs` and rewriting the draw loop using Vello's vector operations (with `vello::Scene::fill` and `kurbo::RoundedRect`), we will establish a pure GPU-accelerated path.

## 3. Caveats
- We did not compile with actual GPU devices present. WGPU adapter and device creation might fail on systems with incompatible graphics drivers.
- We assume `vello` 0.9.0's public API behaves compatibly with `pollster` blocking and WGPU initialization.

## 4. Conclusion
`gui_vello.rs` is currently in a broken, half-refactored state: the backend worker generates `vello::Scene`s, but the frontend GUI thread is stuck with software rendering logic from `gui.rs`.
The refactoring requires:
1. Cleaning up `App`'s fields to use `vello::wgpu` and removing `softbuffer`.
2. Initializing WGPU adapter/device/queue and Vello renderer in `resumed`.
3. Implementing `draw_text_to_scene` using `ab_glyph` font outline rendering.
4. Rewriting `App::draw` and splash screens to append vector shapes/paths directly to a `vello::Scene` instead of blitting pixels.
5. Updating `main.rs` to start the fixed `gui_vello::Gui`.

## 5. Verification Method
1. Compile the workspace using `cargo check` (or the local cargo binary path).
2. The compilation should succeed with no errors when targeting `gui_vello.rs`.
3. Run `cargo run <pdf-file>` to verify the GUI displays PDF pages and overlays correctly on the GPU.
