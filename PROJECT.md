# Project: ufreader GPU Migration

## Architecture
- `src/main.rs`: Entry point. Currently runs `gui::Gui`.
- `src/gui.rs`: Software renderer GUI using `tiny-skia` and `softbuffer`.
- `src/gui_vello.rs`: In-progress GPU-accelerated GUI using `vello` and `wgpu`.
- Data flow for rendering:
  - Text parser extract glyphs.
  - Worker thread converts glyphs into vector curves/scenes.
  - UI rendering loop presents scenes on `wgpu` surface via Vello.

## Code Layout
- `src/main.rs` - Application entry point.
- `src/gui_vello.rs` - The target module to fix compilation and finish implementation.
- `src/parser.rs` - PDF parser.
- `src/interpreter.rs` - PDF content stream interpreter.
- `src/object.rs` - PDF object representations.

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| 1 | Exploration & Analysis | Identify compile errors in `gui_vello.rs` and plan exact types/API conversions | None | DONE |
| 2 | Compilation Fix & Vector Refactoring | Resolve R1 (dependencies/types) and R2 (vector rendering worker path using `vello::Scene` and `kurbo`) | M1 | IN_PROGRESS |
| 3 | GPU presentation UI finalization | Resolve R3 (remove softbuffer, render scene to wgpu surface using Vello) | M2 | IN_PROGRESS |
| 4 | Verification & Quality Review | Conduct independent code reviews, run tests, verify functionality | M3 | PLANNED |
| 5 | Integrity Audit | Run Forensic Integrity Auditor checks to verify clean implementation | M4 | PLANNED |

## Interface Contracts
### Worker Thread ↔ GUI Thread
- Old channel payload: Softbuffer/Pixmap pixel buffers.
- New channel payload: Rendered `vello::Scene` objects (or serialized scene data) to render directly via GPU.
