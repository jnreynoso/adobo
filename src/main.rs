#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod object;
pub mod parser;
pub mod gui;
pub mod gui_vello;
pub mod interpreter;

pub mod db;

use std::env;
use parser::Parser;
use gui_vello::Gui;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut pdf_path = String::new();
    let mut pages_info = Vec::new();
    let mut page_count = 0;
    let mut kill_pid = None;
    let mut pdf_title = "Unknown".to_string();
    let mut pdf_author = "Unknown".to_string();

    for arg in args.iter().skip(1) {
        if arg.starts_with("--kill-pid=") {
            if let Ok(pid) = arg[11..].parse::<u32>() {
                kill_pid = Some(pid);
            }
        } else {
            pdf_path = arg.clone();
        }
    }

    if !pdf_path.is_empty() {
        println!("File: {}", pdf_path);
        
        let path_to_save = if let Ok(abs_path) = std::fs::canonicalize(&pdf_path) {
            if let Some(abs_str) = abs_path.to_str() {
                abs_str.trim_start_matches(r#"\\?\"#).to_string()
            } else {
                pdf_path.clone()
            }
        } else {
            pdf_path.clone()
        };
        gui_vello::add_recent_file(&path_to_save);

        let mut parser = match Parser::new(&pdf_path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to open PDF: {}", e);
                std::process::exit(1);
            }
        };

        if let Err(e) = parser.parse_metadata() {
            eprintln!("Failed to parse PDF metadata: {}", e);
            std::process::exit(1);
        }

        page_count = parser.get_page_count().unwrap_or(0);
        println!("Page Count: {}", page_count);
        
        pdf_title = parser.get_title().unwrap_or_else(|_| "Unknown".to_string());
        pdf_author = parser.get_author().unwrap_or_else(|_| "Unknown".to_string());
        println!("Title: {}", pdf_title);
        println!("Author: {}", pdf_author);

        println!("\nAnalyzing page dimensions...");
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
    
    let gui = Gui::new(pdf_path.clone(), pdf_title, pdf_author, pages_info, logo_rgba, logo_w, logo_h, window_icon, kill_pid);
    if let Err(e) = gui.run() {
        eprintln!("GUI Error: {}", e);
    }
}

fn load_logo_and_icon() -> (Option<Vec<u8>>, u32, u32, Option<winit::window::Icon>) {
    match tiny_skia::Pixmap::load_png("assets/logo.png") {
        Ok(pixmap) => {
            let logo_w = pixmap.width();
            let logo_h = pixmap.height();
            let mut logo_rgba = pixmap.data().to_vec();

            // Make white background transparent
            for pixel in logo_rgba.chunks_exact_mut(4) {
                let a = pixel[3] as f32;
                if a > 0.0 {
                    let r_u = pixel[0] as f32 * 255.0 / a;
                    let g_u = pixel[1] as f32 * 255.0 / a;
                    let b_u = pixel[2] as f32 * 255.0 / a;
                    
                    let dr = 255.0 - r_u;
                    let dg = 255.0 - g_u;
                    let db = 255.0 - b_u;
                    let dist = (dr*dr + dg*dg + db*db).sqrt();
                    
                    let threshold1 = 25.0; // Pure white and very close
                    let threshold2 = 90.0; // Transition zone
                    
                    let alpha_factor = if dist < threshold1 {
                        0.0
                    } else if dist < threshold2 {
                        let t = (dist - threshold1) / (threshold2 - threshold1);
                        t * t * (3.0 - 2.0 * t) // smoothstep
                    } else {
                        1.0
                    };
                    
                    if alpha_factor < 1.0 {
                        let new_a = a * alpha_factor;
                        pixel[0] = (r_u * new_a / 255.0) as u8;
                        pixel[1] = (g_u * new_a / 255.0) as u8;
                        pixel[2] = (b_u * new_a / 255.0) as u8;
                        pixel[3] = new_a as u8;
                    }
                }
            }

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
        }
        Err(e) => {
            eprintln!("Failed to load assets/logo.png: {}", e);
            (None, 0, 0, None)
        }
    }
}
