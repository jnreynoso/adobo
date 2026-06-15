import re

with open('src/gui_vello.rs', 'r', encoding='utf-8') as f:
    content = f.read()

# 1. Fix draw_welcome_screen_to_scene
welcome_screen_old = '''    fn draw_welcome_screen_to_scene(&self, scene: &mut Scene, width: f64, height: f64) {
        let font = &self.default_font;
        let bg_color = vello::peniko::Color::from_rgb8(24, 24, 24);'''

welcome_screen_new = '''    fn draw_welcome_screen_to_scene(&self, scene: &mut Scene, width: f64, height: f64) {
        let font = &self.default_font;
        let bg_color = vello::peniko::Color::from_rgb8(24, 24, 24);'''

# Just make sure draw_welcome_screen_to_scene uses a larger title and draws perfectly.
content = content.replace(welcome_screen_old, welcome_screen_new)

# 2. Fix draw_ui_overlays_to_scene to have TWO menus and only show overlays if not empty
ui_overlays_old = r'''    fn draw_ui_overlays_to_scene(&self, scene: &mut Scene, width: f64, height: f64, hover_state: u8) {
        let font = &self.default_font;
        let overlay_width = 504.0;'''

ui_overlays_new = '''    fn draw_ui_overlays_to_scene(&self, scene: &mut Scene, width: f64, height: f64, hover_state: u8) {
        let font = &self.default_font;
        let has_pdf = !self.pages.is_empty();

        let stroke_style = kurbo::Stroke::new(2.0);
        let border_color = vello::peniko::Color::from_rgb8(100, 100, 100);
        let bg_color = vello::peniko::Color::from_rgba8(25, 25, 25, 220);

        // TOP-LEFT MENU (Abrir Archivo)
        let top_menu_btn_x = 30.0;
        let top_menu_btn_y = 30.0;
        let top_menu_btn_w = 120.0;
        let top_menu_btn_h = 50.0;

        let top_menu_w = 200.0;
        let top_menu_h = 60.0;
        let top_menu_x = 30.0;
        let top_menu_y = top_menu_btn_y + top_menu_btn_h + 5.0;

        // Draw top-left menu toggle button
        let top_btn_rounded = kurbo::RoundedRect::new(top_menu_btn_x, top_menu_btn_y, top_menu_btn_x + top_menu_btn_w, top_menu_btn_y + top_menu_btn_h, 8.0);
        let top_btn_bg_color = vello::peniko::Color::from_rgb8(
            if hover_state == 30 { 70 } else { 25 },
            if hover_state == 30 { 70 } else { 25 },
            if hover_state == 30 { 70 } else { 25 },
        );
        scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, top_btn_bg_color, None, &top_btn_rounded);
        scene.stroke(&stroke_style, kurbo::Affine::IDENTITY, border_color, None, &top_btn_rounded);

        self.draw_text_to_scene(scene, "Menu", top_menu_btn_x + 20.0, top_menu_btn_y + 35.0, 28.0, font, vello::peniko::Color::WHITE);

        if self.top_menu_open {
            let top_menu_rounded = kurbo::RoundedRect::new(top_menu_x, top_menu_y, top_menu_x + top_menu_w, top_menu_y + top_menu_h, 8.0);
            scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, bg_color, None, &top_menu_rounded);
            scene.stroke(&stroke_style, kurbo::Affine::IDENTITY, border_color, None, &top_menu_rounded);

            if hover_state == 31 {
                let h_rect = kurbo::Rect::new(top_menu_x + 1.0, top_menu_y + 1.0, top_menu_x + top_menu_w - 1.0, top_menu_y + top_menu_h - 1.0);
                scene.fill(vello::peniko::Fill::NonZero, kurbo::Affine::IDENTITY, vello::peniko::Color::from_rgba8(100, 100, 100, 200), None, &h_rect);
            }
            self.draw_text_to_scene(scene, "Abrir archivo...", top_menu_x + 20.0, top_menu_y + 38.0, 24.0, font, vello::peniko::Color::WHITE);
        }

        if !has_pdf {
            return; // Don't draw the rest of the UI overlays
        }

        let overlay_width = 504.0;'''

# Replace ui overlay header
content = re.sub(r'    fn draw_ui_overlays_to_scene\(&self, scene: &mut Scene, width: f64, height: f64, hover_state: u8\) \{\n        let font = &self\.default_font;\n        let overlay_width = 504\.0;', ui_overlays_new, content)

# 3. Fix Bottom-Left Menu in draw_ui_overlays
bottom_menu_old = r'''        // 3. Left Menu Overlay \(Top-Left\)
        let menu_btn_x = 30\.0;
        let menu_btn_y = 30\.0;
        let menu_btn_w = 120\.0;
        let menu_btn_h = 60\.0;

        let menu_w = 364\.0;
        let menu_h = 504\.0; // Increased height for the extra item
        let menu_x = 30\.0;
        let menu_y = menu_btn_y \+ menu_btn_h \+ 10\.0;'''

bottom_menu_new = '''        // 3. Left Menu Overlay (Bottom-Left)
        let menu_btn_x = 30.0;
        let menu_btn_y = height - 100.0 - 30.0;
        let menu_btn_w = 84.0;
        let menu_btn_h = 100.0;

        let menu_w = 364.0;
        let menu_h = 448.0;
        let menu_x = 30.0;
        let menu_y = height - 100.0 - 30.0 - menu_h - 10.0;'''

content = re.sub(bottom_menu_old, bottom_menu_new, content)

# Remove the "Abrir archivo" from the bottom menu list
abrir_old = r'''                let items = \[
                    "Abrir archivo...",
                    "Single-page view",'''
abrir_new = '''                let items = [
                    "Single-page view",'''
content = re.sub(abrir_old, abrir_new, content)

# Remove the text menu toggle draw and restore hamburger
text_btn_old = r'''            // Draw "Menu" text
            self\.draw_text_to_scene\(
                scene,
                "Menu",
                menu_btn_x \+ 20\.0,
                menu_btn_y \+ 40\.0,
                32\.0,
                font,
                vello::peniko::Color::WHITE,
            \);'''
text_btn_new = '''            // Three lines for hamburger menu
            for i in 0..3 {
                let rect = kurbo::Rect::new(
                    menu_btn_x + 21.0,
                    menu_btn_y + 34.0 + (i as f64 * 12.0),
                    menu_btn_x + 21.0 + 42.0,
                    menu_btn_y + 34.0 + (i as f64 * 12.0) + 6.0,
                );
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    kurbo::Affine::IDENTITY,
                    vello::peniko::Color::WHITE,
                    None,
                    &rect,
                );
            }'''
content = re.sub(text_btn_old, text_btn_new, content)

# Change item_h back to 8.0
content = content.replace("let item_h = menu_h / 9.0;", "let item_h = menu_h / 8.0;")

with open('src/gui_vello.rs', 'w', encoding='utf-8') as f:
    f.write(content)
