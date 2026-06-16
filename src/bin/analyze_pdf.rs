use ufreader::parser::Parser;
use ufreader::interpreter::{Interpreter, DrawCommand};

fn main() {
    let path = "ontologia.pdf";
    let mut parser = match Parser::new(path) {
        Ok(p) => p,
        Err(e) => {
            println!("Failed to parse PDF: {:?}", e);
            return;
        }
    };
    
    if let Err(e) = parser.parse_metadata() {
        println!("Failed to parse PDF metadata: {}", e);
        return;
    }
    
    let page_count = parser.get_page_count().unwrap_or(0);
    println!("Total pages: {}", page_count);
    
    if page_count > 0 {
        let page_idx = 0;
        let images = parser.get_page_images(page_idx).unwrap_or_default();
        
        println!("Found {} images:", images.len());
        for (name, img) in &images {
            println!("  - Image '{}': {}x{}, filter: '{}', color_space: '{}', bits: {}",
                name, img.width, img.height, img.filter, img.color_space, img.bits_per_component);
        }
        
        println!("Running interpreter for page {}...", page_idx);
        let interpreter = Interpreter::new(std::collections::HashMap::new(), std::collections::HashMap::new(), std::collections::HashMap::new(), std::collections::HashMap::new());
        let content = parser.get_page_content(page_idx).unwrap_or(vec![]);
        println!("Page content (first 500 bytes):\n{}", String::from_utf8_lossy(&content).chars().take(500).collect::<String>());
        let rect = parser.get_page_rect(page_idx).unwrap_or(ufreader::parser::PageRect{x:0.0, y:0.0, width: 595.0, height: 842.0});
        
        if let Some(commands) = interpreter.process(page_idx, &content, rect, None, None, &images) {
            println!("Found {} draw commands.", commands.len());
            for cmd in commands.iter() {
                if let DrawCommand::Image { name, transform, .. } = cmd {
                    println!("  -> DrawCommand::Image '{}' at {:?}", name, transform);
                }
            }
        } else {
            println!("Interpreter returned None (epoch mismatch or empty)");
        }
    }
}
