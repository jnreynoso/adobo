# Plan - GPU Migration and Compilation Fix

This plan decomposes the task of resolving compilation errors in `gui_vello.rs` and completing the GPU migration.

## Steps

### Step 1: Investigation & Codebase Analysis
- **Goal**: Analyze existing compilation errors, understand Vello/wgpu usage in `gui_vello.rs`, and determine how to map old tiny-skia types to vello/kurbo.
- **Agent**: `explorer_1` (teamwork_preview_explorer)
- **Verification**: List all cargo check compilation errors and detailed file structure changes needed.

### Step 2: Fix Compilation Errors & Refactor
- **Goal**: Fix compilation errors in `gui_vello.rs` and related modules. Eliminate softbuffer/tiny-skia references. Correct kurbo/ab_glyph vector conversions.
- **Agent**: `worker_1` (teamwork_preview_worker)
- **Verification**: `cargo check` and `cargo build` compile successfully.

### Step 3: Code Review & Quality Assurance
- **Goal**: Review the worker's changes for correctness, API conventions, and proper integration with Vello.
- **Agent**: `reviewer_1` (teamwork_preview_reviewer)
- **Verification**: Code review pass and validation of geometry representation correctness.

### Step 4: Verification & Integrity Audit
- **Goal**: Validate functionality and run integrity forensics checks to verify that no shortcuts or fake implementations were used.
- **Agent**: `auditor_1` (teamwork_preview_auditor)
- **Verification**: Forensic audit passes cleanly.
