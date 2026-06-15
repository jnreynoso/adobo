fn main() {
    match tiny_skia::Pixmap::load_png("assets/logo.png") {
        Ok(pixmap) => {
            println!("Success! Width: {}, Height: {}", pixmap.width(), pixmap.height());
        }
        Err(e) => {
            println!("Failed to load PNG: {}", e);
        }
    }
}
