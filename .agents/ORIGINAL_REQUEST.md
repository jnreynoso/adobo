# Original User Request

## 2026-06-10T18:26:34Z

# Teamwork Project Prompt — Draft

> Status: Launched
> Goal: Resolve the 127 compilation errors in `gui_vello.rs` and complete the GPU migration.

Resolver los 127 errores de compilación actuales en `src/gui_vello.rs` para completar la migración gráfica de UfReader de renderizado por software (`tiny-skia`) a aceleración por hardware (`vello` y `wgpu`).

Working directory: C:/Users/jreyn/OneDrive/Documents/Projects/ufreader
Integrity mode: development

## Requirements

### R1. Resolución de Errores de Tipado y Dependencias
Reparar todos los errores de sintaxis, variables sin usar, importaciones faltantes de `wgpu` y tipos no encontrados (ej. `Rect`, `Transform`, `Paint`) que quedaron del código viejo de `tiny-skia` en el archivo `gui_vello.rs`.

### R2. Refactorización Vectorial del Worker
Asegurarse de que el `run_worker_thread` construya correctamente las curvas `kurbo::BezPath` a partir de `ab_glyph`, las empaquete en un `vello::Scene` y las envíe exitosamente por el canal hacia la UI, descartando por completo el uso de `Pixmap`.

### R3. Finalización de la UI y WGPU
Garantizar que el método `draw` y la inicialización `resumed` utilicen correctamente el `Renderer` de Vello y la superficie gráfica de `wgpu` para dibujar las escenas en pantalla, eliminando cualquier rastro de `softbuffer`.

## Acceptance Criteria

### Compilación Exitosa
- [ ] Ejecutar `cargo check` no debe arrojar ningún error relacionado a `gui_vello.rs`.
- [ ] Ejecutar `cargo build` compila un binario funcional sin errores críticos.

### Verificación Gráfica
- [ ] El código de Rust demuestra un uso exclusivo de `vello::Scene`, `wgpu` y `kurbo` para la geometría, sin importar tipos o buffers de renderizado de `tiny-skia`.
