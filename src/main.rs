mod object;
pub mod parser;
pub mod gui;
pub mod gui_vello;
pub mod interpreter;

use std::env;
use parser::Parser;
use gui_vello::Gui;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <pdf-file>", args[0]);
        std::process::exit(1);
    }

    let pdf_path = &args[1];
    let mut parser = match Parser::new(pdf_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error opening PDF: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = parser.parse_metadata() {
        eprintln!("Error parsing metadata: {}", e);
        std::process::exit(1);
    }

    let page_count = parser.get_page_count().unwrap_or(0);
    let author = parser.get_author().unwrap_or_else(|_| "Unknown".to_string());

    println!(" __    __  ______  ______  ______  ______  _____   ______  ______    ");
    println!("/\\ \\  /\\ \\/\\  ___\\/\\  == \\/\\  ___\\/\\  __ \\/\\  __-.\\/\\  ___\\/\\  == \\   ");
    println!("\\ \\ \\_\\ \\ \\ \\  __\\\\ \\  __<\\ \\  __\\\\ \\  __ \\ \\ \\/\\ \\ \\  __\\\\ \\  __<   ");
    println!(" \\ \\_____\\ \\ \\_\\   \\ \\_\\ \\_\\ \\_____\\ \\_\\ \\_\\ \\____-\\ \\_____\\ \\_\\ \\_\\ ");
    println!("  \\/_____/  \\/_/    \\/_/ /_/\\/_____/\\/_/\\/_/\\/____/ \\/_____/\\/_/ /_/ ");
    println!("---------------------------------------------------------------------");
    println!("File: {}", pdf_path);
    println!("Page Count: {}", page_count);
    println!("Author: {}\n", author);

    let mut pages_info = Vec::new();

    if page_count > 0 {
        println!("Analyzing page dimensions...");
        let mut current_top_y = 0.0;
        for page_idx in 0..page_count as usize {
            let page_rect = parser.get_page_rect(page_idx).unwrap_or(parser::PageRect {
                x: 0.0,
                y: 0.0,
                width: 595.0,
                height: 842.0,
            });
            pages_info.push(gui_vello::PageInfo {
                width: page_rect.width,
                height: page_rect.height,
                top_y: current_top_y,
                center_x_offset: 0.0,
            });
            current_top_y += page_rect.height + 20.0;
        }
    }

    println!("\nLaunching GUI...");
    let (logo_rgba, logo_w, logo_h, window_icon) = load_logo_and_icon();
    let gui = Gui::new(pdf_path.clone(), pages_info, logo_rgba, logo_w, logo_h, window_icon);
    if let Err(e) = gui.run() {
        eprintln!("GUI Error: {}", e);
    }
}

fn load_logo_and_icon() -> (Option<Vec<u8>>, u32, u32, Option<winit::window::Icon>) {
    if let Ok(pixmap) = tiny_skia::Pixmap::load_png("logo.png") {
        let logo_w = pixmap.width();
        let logo_h = pixmap.height();
        let logo_rgba = pixmap.data().to_vec();

        // Generate window icon (64x64)
        let target_size = 64;
        let icon = if let Some(mut resized) = tiny_skia::Pixmap::new(target_size, target_size) {
            let sx = target_size as f32 / logo_w as f32;
            let sy = target_size as f32 / logo_h as f32;
            let transform = tiny_skia::Transform::from_scale(sx, sy);
            resized.draw_pixmap(
                0, 0,
                pixmap.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                transform,
                None
            );
            let mut rgba = resized.data().to_vec();
            for pixel in rgba.chunks_exact_mut(4) {
                let a = pixel[3];
                if a > 0 && a < 255 {
                    pixel[0] = ((pixel[0] as u16 * 255) / a as u16) as u8;
                    pixel[1] = ((pixel[1] as u16 * 255) / a as u16) as u8;
                    pixel[2] = ((pixel[2] as u16 * 255) / a as u16) as u8;
                }
            }
            winit::window::Icon::from_rgba(rgba, target_size, target_size).ok()
        } else {
            None
        };

        (Some(logo_rgba), logo_w, logo_h, icon)
    } else {
        (None, 0, 0, None)
    }
}
