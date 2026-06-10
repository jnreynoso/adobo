# Context

The application `ufreader` is a PDF reader built in Rust. It has an existing software-based GUI (`gui.rs`) using `tiny-skia` and `softbuffer`.
There is an ongoing migration to hardware-accelerated GPU rendering using `vello` and `wgpu` (`gui_vello.rs`).
However, `gui_vello.rs` has ~127 compilation errors. These errors stem from:
1. Missing or incompatible imports (e.g. types like `Rect`, `Transform`, `Paint` from `tiny-skia` still referenced, or mismatch with `kurbo`/`vello` equivalents).
2. Unfinished vector refactoring of the worker thread (`run_worker_thread`), which needs to generate `kurbo::BezPath` from glyphs and output a `vello::Scene` instead of using `Pixmap`.
3. Incomplete UI integration with `wgpu` and `vello::Renderer` (removing `softbuffer` completely).
