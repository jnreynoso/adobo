fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/logo.ico");
        if let Err(e) = res.compile() {
            eprintln!("Error compiling windows resources: {}", e);
            std::process::exit(1);
        }
    }
}
