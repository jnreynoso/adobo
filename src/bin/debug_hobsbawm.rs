use std::sync::atomic::AtomicUsize;

fn main() {
    let pdf_path = "Eric Hobsbawm - Historia del Siglo XX.pdf";
    let mut parser = match ufreader::parser::Parser::new(pdf_path) {
        Ok(p) => p,
        Err(e) => {
            println!("Error opening: {}", e);
            return;
        }
    };
    if let Err(e) = parser.parse_metadata() {
        println!("Error parsing metadata: {}", e);
        return;
    }

    let count = parser.get_page_count().unwrap_or(0);
    println!("Pages: {}", count);

    let default_font = std::sync::Arc::new(
        ab_glyph::FontVec::try_from_vec(std::fs::read("C:/Windows/Fonts/arial.ttf").unwrap())
            .unwrap(),
    );

    let pdf_fonts = parser.find_fonts();
    let mut font_encodings = std::collections::HashMap::new();
    let mut font_widths = std::collections::HashMap::new();
    let mut font_names = std::collections::HashMap::new();

    // Simplistic mock for the interpreter
    let interpreter = ufreader::interpreter::Interpreter::new(
        font_encodings,
        font_widths,
        std::collections::HashMap::new(),
        font_names,
    );

    let epoch = AtomicUsize::new(0);

    for i in 0..10.min(count as usize) {
        let content = parser.get_page_content(i).unwrap_or_default();
        println!("Page {} content length: {}", i, content.len());

        let page_rect = parser
            .get_page_rect(i)
            .unwrap_or(ufreader::parser::PageRect {
                x: 0.0,
                y: 0.0,
                width: 595.0,
                height: 842.0,
            });

        if let Some(cmds) = interpreter.process(i, &content, page_rect, Some(&epoch), Some(0)) {
            println!("Page {} commands: {}", i, cmds.len());
            let mut text_count = 0;
            let mut total_chars = 0;
            let mut outline_count = 0;
            let mut has_glyphs = false;
            let mut builder = kurbo::BezPath::new();
            use ab_glyph::Font;
            for cmd in cmds {
                if let ufreader::interpreter::DrawCommand::Text { chars, size, .. } = cmd {
                    text_count += 1;
                    total_chars += chars.len();

                    let font = &default_font;
                    let scale_factor = size / font.units_per_em().unwrap_or(1000.0);
                    for (c, x, expected_w) in chars {
                        let glyph_id = font.glyph_id(c);
                        if let Some(outline) = font.outline(glyph_id) {
                            outline_count += 1;
                            has_glyphs = true;
                            let actual_w = font.h_advance_unscaled(glyph_id) * scale_factor;
                            let h_squeeze = if actual_w > 0.0 && expected_w > 0.0 {
                                expected_w / actual_w
                            } else {
                                1.0
                            };
                            let h_squeeze = h_squeeze.clamp(0.4, 2.5);

                            let mut last_point: Option<ab_glyph::Point> = None;
                            for curve in outline.curves {
                                let start_p = match curve {
                                    ab_glyph::OutlineCurve::Line(p1, _) => p1,
                                    ab_glyph::OutlineCurve::Quad(p1, _, _) => p1,
                                    ab_glyph::OutlineCurve::Cubic(p1, _, _, _) => p1,
                                };
                                let is_new_contour = match last_point {
                                    Some(lp) => {
                                        (start_p.x - lp.x).abs() > 0.001
                                            || (start_p.y - lp.y).abs() > 0.001
                                    }
                                    None => true,
                                };
                                if is_new_contour {
                                    builder.move_to((
                                        (x + start_p.x * scale_factor * h_squeeze) as f64,
                                        (start_p.y * scale_factor) as f64,
                                    ));
                                }
                                match curve {
                                    ab_glyph::OutlineCurve::Line(_, p2) => {
                                        builder.line_to((
                                            (x + p2.x * scale_factor * h_squeeze) as f64,
                                            (p2.y * scale_factor) as f64,
                                        ));
                                        last_point = Some(p2);
                                    }
                                    ab_glyph::OutlineCurve::Quad(_, p2, p3) => {
                                        builder.quad_to(
                                            (
                                                (x + p2.x * scale_factor * h_squeeze) as f64,
                                                (p2.y * scale_factor) as f64,
                                            ),
                                            (
                                                (x + p3.x * scale_factor * h_squeeze) as f64,
                                                (p3.y * scale_factor) as f64,
                                            ),
                                        );
                                        last_point = Some(p3);
                                    }
                                    ab_glyph::OutlineCurve::Cubic(_, p2, p3, p4) => {
                                        builder.curve_to(
                                            (
                                                (x + p2.x * scale_factor * h_squeeze) as f64,
                                                (p2.y * scale_factor) as f64,
                                            ),
                                            (
                                                (x + p3.x * scale_factor * h_squeeze) as f64,
                                                (p3.y * scale_factor) as f64,
                                            ),
                                            (
                                                (x + p4.x * scale_factor * h_squeeze) as f64,
                                                (p4.y * scale_factor) as f64,
                                            ),
                                        );
                                        last_point = Some(p4);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            println!(
                "Page {} text cmds: {}, chars: {}, outlines: {}",
                i, text_count, total_chars, outline_count
            );
        } else {
            println!("Page {} process failed", i);
        }
    }
}
