use std::collections::HashMap;
use encoding_rs::{WINDOWS_1252, MACINTOSH};

#[derive(Clone)]
pub struct GraphicsState {
    pub x: f32,
    pub y: f32,
    pub line_x: f32,
    pub line_y: f32,
    pub font_size: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub ctm_scale_x: f32,
    pub ctm_scale_y: f32,
    pub current_font: String,
    pub char_spacing: f32,
    pub word_spacing: f32,
    pub horiz_scaling: f32,
    pub text_rise: f32,
}

impl GraphicsState {
    pub fn new() -> Self {
        Self {
            x: 0.0, y: 0.0,
            line_x: 0.0, line_y: 0.0,
            font_size: 12.0,
            scale_x: 1.0, scale_y: 1.0,
            ctm_scale_x: 1.0, ctm_scale_y: 1.0,
            current_font: String::new(),
            char_spacing: 0.0,
            word_spacing: 0.0,
            horiz_scaling: 100.0,
            text_rise: 0.0,
        }
    }
}

#[derive(Clone)]
pub enum PdfEncoding {
    WinAnsi,
    MacRoman,
    Custom(HashMap<u32, char>),
}

#[derive(Clone)]
pub struct Interpreter {
    pub font_encodings: HashMap<String, PdfEncoding>,
    pub font_widths: HashMap<String, HashMap<u32, f32>>,
    pub fallback_char_widths: HashMap<char, f32>,
    pub font_names: HashMap<String, String>,
}

impl Interpreter {
    pub fn new(
        font_encodings: HashMap<String, PdfEncoding>,
        font_widths: HashMap<String, HashMap<u32, f32>>,
        fallback_char_widths: HashMap<char, f32>,
        font_names: HashMap<String, String>,
    ) -> Self {
        Self { font_encodings, font_widths, fallback_char_widths, font_names }
    }

    fn decode_and_advance(&self, bytes: &[u8], state: &mut GraphicsState) -> Vec<(char, f32, f32)> {
        let mut result = Vec::new();
        let mut i = 0;
        
        let enc = self.font_encodings.get(&state.current_font);
        let widths = self.font_widths.get(&state.current_font);

        let is_2_byte = if bytes.starts_with(&[0xFE, 0xFF]) {
            i += 2;
            true
        } else {
            match enc {
                Some(PdfEncoding::Custom(_)) if bytes.len() % 2 == 0 && bytes.len() > 0 => true,
                _ => false,
            }
        };

        let th = state.horiz_scaling / 100.0;
        let effective_scale_x = state.scale_x * state.ctm_scale_x * th;

        while i < bytes.len() {
            let char_code: u32;
            let c: char;
            let byte_len: usize;

            if is_2_byte && i + 1 < bytes.len() {
                char_code = ((bytes[i] as u32) << 8) | (bytes[i+1] as u32);
                byte_len = 2;
                if let Some(PdfEncoding::Custom(map)) = enc {
                    c = *map.get(&char_code).unwrap_or(&' ');
                } else {
                    c = std::char::from_u32(char_code).unwrap_or(' ');
                }
            } else {
                char_code = bytes[i] as u32;
                byte_len = 1;
                if let Some(PdfEncoding::WinAnsi) = enc {
                    let (res, _, _) = WINDOWS_1252.decode(&bytes[i..i+1]);
                    c = res.chars().next().unwrap_or(' ');
                } else if let Some(PdfEncoding::MacRoman) = enc {
                    let (res, _, _) = MACINTOSH.decode(&bytes[i..i+1]);
                    c = res.chars().next().unwrap_or(' ');
                } else if let Some(PdfEncoding::Custom(map)) = enc {
                    c = *map.get(&char_code).unwrap_or(&(bytes[i] as char));
                } else {
                    let (res, _, _) = WINDOWS_1252.decode(&bytes[i..i+1]);
                    c = res.chars().next().unwrap_or(' ');
                }
            }

            let w = widths.and_then(|w| w.get(&char_code)).unwrap_or_else(|| {
                self.fallback_char_widths.get(&c).unwrap_or(&500.0)
            });
            let expected_width = (w / 1000.0) * state.font_size * effective_scale_x;

            if c != '\u{0000}' && c != '\u{0001}' && c != '\u{0002}' && c != '\u{0003}' && c != '\u{0008}' {
                result.push((c, state.line_x + state.x, expected_width));
            }
            
            state.x += expected_width;
            state.x += state.char_spacing * effective_scale_x;
            if char_code == 32 { state.x += state.word_spacing * effective_scale_x; }
            
            i += byte_len;
        }
        result
    }

    fn unescape(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                match bytes[i+1] {
                    b'n' => { out.push(b'\n'); i += 2; },
                    b'r' => { out.push(b'\r'); i += 2; },
                    b't' => { out.push(b'\t'); i += 2; },
                    b'b' => { out.push(8); i += 2; },
                    b'f' => { out.push(12); i += 2; },
                    b'(' => { out.push(b'('); i += 2; },
                    b')' => { out.push(b')'); i += 2; },
                    b'\\' => { out.push(b'\\'); i += 2; },
                    b'0'..=b'7' => {
                        let mut octal = (bytes[i+1] - b'0') as u32;
                        let mut count = 1;
                        while count < 3 && i + 1 + count < bytes.len() && bytes[i+1+count].is_ascii_digit() {
                            octal = octal * 8 + (bytes[i+1+count] - b'0') as u32;
                            count += 1;
                        }
                        out.push(octal as u8);
                        i += count + 1;
                    }
                    _ => { out.push(bytes[i+1]); i += 2; },
                }
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        out
    }

    pub fn process(
        &self,
        page_idx: usize,
        content_stream: &[u8],
        page_rect: crate::parser::PageRect,
        render_epoch: Option<&std::sync::atomic::AtomicUsize>,
        current_epoch: Option<usize>,
    ) -> Option<Vec<DrawCommand>> {
        let mut commands = Vec::new();
        let mut state = GraphicsState::new();
        let mut state_stack = Vec::new();
        let mut tokens: Vec<Vec<u8>> = Vec::new();
        let mut i = 0;
        let mut loop_counter = 0;
        
        while i < content_stream.len() {
            loop_counter += 1;
            if loop_counter % 200 == 0 {
                if let (Some(epoch), Some(curr)) = (render_epoch, current_epoch) {
                    if epoch.load(std::sync::atomic::Ordering::Relaxed) != curr {
                        return None;
                    }
                }
            }
            let b = content_stream[i];
            match b {
                b'(' => {
                    let mut s = vec![b'('];
                    i += 1;
                    let mut balance = 1;
                    while i < content_stream.len() {
                        let nb = content_stream[i];
                        s.push(nb);
                        if nb == b')' {
                            let mut bslashes = 0;
                            let mut j = s.len() as i32 - 2;
                            while j >= 0 && s[j as usize] == b'\\' { bslashes += 1; j -= 1; }
                            if bslashes % 2 == 0 { balance -= 1; if balance == 0 { break; } }
                        } else if nb == b'(' {
                            let mut bslashes = 0;
                            let mut j = s.len() as i32 - 2;
                            while j >= 0 && s[j as usize] == b'\\' { bslashes += 1; j -= 1; }
                            if bslashes % 2 == 0 { balance += 1; }
                        }
                        i += 1;
                    }
                    tokens.push(s);
                }
                b'<' => {
                    let mut s = vec![b'<'];
                    i += 1;
                    while i < content_stream.len() {
                        let nb = content_stream[i];
                        s.push(nb);
                        if nb == b'>' { break; }
                        i += 1;
                    }
                    tokens.push(s);
                }
                b'[' | b']' | b'/' => tokens.push(vec![b]),
                _ if b.is_ascii_whitespace() => {}
                _ => {
                    let mut s = vec![b];
                    i += 1;
                    while i < content_stream.len() {
                        let nb = content_stream[i];
                        if nb.is_ascii_whitespace() || b"()[]<>/".contains(&nb) { i -= 1; break; }
                        s.push(nb);
                        i += 1;
                    }
                    tokens.push(s);
                }
            }
            i += 1;
        }

        let mut idx = 0;
        let mut loop_counter2 = 0;
        while idx < tokens.len() {
            loop_counter2 += 1;
            if loop_counter2 % 200 == 0 {
                if let (Some(epoch), Some(curr)) = (render_epoch, current_epoch) {
                    if epoch.load(std::sync::atomic::Ordering::Relaxed) != curr {
                        return None;
                    }
                }
            }
            let token_str = String::from_utf8_lossy(&tokens[idx]);
            match token_str.as_ref() {
                "q" => {
                    state_stack.push(state.clone());
                }
                "Q" => {
                    if let Some(saved_state) = state_stack.pop() {
                        state = saved_state;
                    }
                }
                "cm" => {
                    if idx >= 6 {
                        let a = String::from_utf8_lossy(&tokens[idx-6]).parse::<f32>().unwrap_or(1.0);
                        let d = String::from_utf8_lossy(&tokens[idx-3]).parse::<f32>().unwrap_or(1.0);
                        state.ctm_scale_x *= if a == 0.0 { 1.0 } else { a.abs() };
                        state.ctm_scale_y *= if d == 0.0 { 1.0 } else { d.abs() };
                    }
                }
                "Tm" => {
                    if idx >= 6 {
                        let a = String::from_utf8_lossy(&tokens[idx-6]).parse::<f32>().unwrap_or(1.0);
                        let d = String::from_utf8_lossy(&tokens[idx-3]).parse::<f32>().unwrap_or(1.0);
                        let e = String::from_utf8_lossy(&tokens[idx-2]).parse::<f32>().unwrap_or(0.0);
                        let f = String::from_utf8_lossy(&tokens[idx-1]).parse::<f32>().unwrap_or(0.0);
                        state.scale_x = if a == 0.0 { 1.0 } else { a.abs() };
                        state.scale_y = if d == 0.0 { 1.0 } else { d.abs() };
                        state.line_x = e;
                        state.line_y = f;
                        state.x = 0.0; state.y = 0.0;
                    }
                }
                "BT" => { 
                    state.x = 0.0; state.y = 0.0; 
                    state.line_x = 0.0; state.line_y = 0.0;
                    state.scale_x = 1.0; state.scale_y = 1.0;
                    // Note: BT does NOT reset CTM, char_spacing, word_spacing, horiz_scaling, or text_rise
                }
                "Td" | "TD" => {
                    if idx >= 2 {
                        let tx = String::from_utf8_lossy(&tokens[idx-2]).parse::<f32>().unwrap_or(0.0);
                        let ty = String::from_utf8_lossy(&tokens[idx-1]).parse::<f32>().unwrap_or(0.0);
                        state.line_x += tx * state.scale_x;
                        state.line_y += ty * state.scale_y;
                        state.x = 0.0; state.y = 0.0;
                    }
                }
                "Tf" => { 
                    if idx >= 2 { 
                        state.current_font = String::from_utf8_lossy(&tokens[idx-2]).trim_start_matches('/').to_string();
                        state.font_size = String::from_utf8_lossy(&tokens[idx-1]).parse::<f32>().unwrap_or(12.0); 
                    } 
                }
                "Tz" => { if idx >= 1 { state.horiz_scaling = String::from_utf8_lossy(&tokens[idx-1]).parse::<f32>().unwrap_or(100.0); } }
                "Ts" => { if idx >= 1 { state.text_rise = String::from_utf8_lossy(&tokens[idx-1]).parse::<f32>().unwrap_or(0.0); } }
                "Tc" => { if idx >= 1 { state.char_spacing = String::from_utf8_lossy(&tokens[idx-1]).parse::<f32>().unwrap_or(0.0); } }
                "Tw" => { if idx >= 1 { state.word_spacing = String::from_utf8_lossy(&tokens[idx-1]).parse::<f32>().unwrap_or(0.0); } }
                "Tj" => {
                    if idx > 0 {
                        let t = &tokens[idx-1];
                        let bytes = if t.starts_with(b"(") { Self::unescape(&t[1..t.len()-1]) }
                        else if t.starts_with(b"<") {
                             let hex = String::from_utf8_lossy(&t[1..t.len()-1]).trim_end_matches('>').to_string();
                             (0..hex.len()).step_by(2).filter_map(|j| {
                                 if j + 1 <= hex.len() { u8::from_str_radix(&hex[j..j+2], 16).ok() } else { None }
                             }).collect()
                        } else { Vec::new() };
                        
                        if !bytes.is_empty() {
                            let mut chars = self.decode_and_advance(&bytes, &mut state);
                            if !chars.is_empty() {
                                for char_info in &mut chars {
                                    char_info.1 -= page_rect.x;
                                }
                                let font_base_name = self.font_names.get(&state.current_font).cloned().unwrap_or_else(|| "Serif".to_string());
                                commands.push(DrawCommand::Text {
                                    chars,
                                    page_idx,
                                    local_y: state.line_y + state.y + state.text_rise - page_rect.y,
                                    size: state.font_size * state.scale_y * state.ctm_scale_y,
                                    font_name: font_base_name,
                                });
                            }
                        }
                    }
                }
                "TJ" => {
                    if idx > 0 && tokens[idx-1] == b"]" {
                        let mut j = idx - 2;
                        while j > 0 && tokens[j] != b"[" { j -= 1; }
                        let mut all_chars = Vec::new();
                        let th = state.horiz_scaling / 100.0;
                        let effective_scale_x = state.scale_x * state.ctm_scale_x * th;

                        for k in j + 1 .. idx - 1 {
                            let t = &tokens[k];
                            if t.starts_with(b"(") || t.starts_with(b"<") {
                                let bytes = if t.starts_with(b"(") { Self::unescape(&t[1..t.len()-1]) }
                                else {
                                     let hex = String::from_utf8_lossy(&t[1..t.len()-1]).trim_end_matches('>').to_string();
                                     (0..hex.len()).step_by(2).filter_map(|m| {
                                         if m + 1 <= hex.len() { u8::from_str_radix(&hex[m..m+2], 16).ok() } else { None }
                                     }).collect()
                                };
                                all_chars.extend(self.decode_and_advance(&bytes, &mut state));
                            } else if let Ok(k_val) = String::from_utf8_lossy(t).parse::<f32>() {
                                state.x -= (k_val / 1000.0) * state.font_size * effective_scale_x;
                            }
                        }
                        if !all_chars.is_empty() {
                            for char_info in &mut all_chars {
                                char_info.1 -= page_rect.x;
                            }
                            let font_base_name = self.font_names.get(&state.current_font).cloned().unwrap_or_else(|| "Serif".to_string());
                            commands.push(DrawCommand::Text {
                                chars: all_chars,
                                page_idx,
                                local_y: state.line_y + state.y + state.text_rise - page_rect.y,
                                size: state.font_size * state.scale_y * state.ctm_scale_y,
                                font_name: font_base_name,
                            });
                        }
                    }
                }
                _ => {}
            }
            idx += 1;
        }
        Some(commands)
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum DrawCommand {
    Text {
        chars: Vec<(char, f32, f32)>,
        page_idx: usize,
        local_y: f32,
        size: f32,
        font_name: String,
    },
}
