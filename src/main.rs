mod object;
mod parser;
mod gui;
mod interpreter;

use std::env;
use parser::Parser;
use gui::Gui;

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
            pages_info.push(gui::PageInfo {
                width: page_rect.width,
                height: page_rect.height,
                top_y: current_top_y,
            });
            current_top_y += page_rect.height + 20.0;
        }
    }

    println!("\nLaunching GUI...");
    let gui = Gui::new(pdf_path.clone(), pages_info);
    if let Err(e) = gui.run() {
        eprintln!("GUI Error: {}", e);
    }
}
