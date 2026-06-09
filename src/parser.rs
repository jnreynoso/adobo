use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Cursor};
use crate::object::PdfObject;
use std::collections::{HashMap, HashSet};
use flate2::read::ZlibDecoder;

#[derive(Debug, Clone, Copy)]
pub struct PageRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}


#[derive(Clone, Debug)]
enum XrefEntry {
    Normal(u64),
    Compressed(u32, u32), // container_id, index
}

pub struct Parser {
    file: File,
    xref: HashMap<u32, XrefEntry>,
    trailer: HashMap<String, PdfObject>,
    visited_offsets: HashSet<u64>,
    resolved_cache: HashMap<u32, PdfObject>,
}

impl Parser {
    pub fn new(path: &str) -> io::Result<Self> {
        let file = File::open(path)?;
        Ok(Parser { 
            file,
            xref: HashMap::new(),
            trailer: HashMap::new(),
            visited_offsets: HashSet::new(),
            resolved_cache: HashMap::new(),
        })
    }

    pub fn find_startxref(&mut self) -> io::Result<u64> {
        let file_size = self.file.metadata()?.len();
        let buffer_size = 1024.min(file_size) as usize;
        let mut buffer = vec![0; buffer_size];
        self.file.seek(SeekFrom::End(-(buffer_size as i64)))?;
        self.file.read_exact(&mut buffer)?;
        let content = String::from_utf8_lossy(&buffer);
        if let Some(pos) = content.rfind("startxref") {
            let offset_part = &content[pos + 9..];
            let offset_str = offset_part.split_whitespace().next().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Could not find offset after startxref")
            })?;
            offset_str.parse::<u64>().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid offset after startxref")
            })
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidData, "startxref not found in the last 1024 bytes"))
        }
    }

    pub fn parse_metadata(&mut self) -> io::Result<()> {
        let startxref = self.find_startxref()?;
        self.visited_offsets.clear();
        self.parse_xref(startxref)?;
        Ok(())
    }

    fn parse_xref(&mut self, offset: u64) -> io::Result<()> {
        if self.visited_offsets.contains(&offset) { return Ok(()); }
        self.visited_offsets.insert(offset);
        self.file.seek(SeekFrom::Start(offset))?;
        let mut head = [0; 4];
        if self.file.read_exact(&mut head).is_err() { return Ok(()); }
        let mut current_trailer = HashMap::new();
        if &head == b"xref" {
            loop {
                let mut line = String::new();
                Self::read_line(&mut self.file, &mut line)?;
                if line.trim() == "trailer" {
                    Self::skip_whitespace(&mut self.file)?;
                    let mut buf = [0; 2];
                    self.file.read_exact(&mut buf)?;
                    if &buf == b"<<" {
                        let trailer_dict = Self::parse_dictionary_content(&mut self.file)?;
                        if let PdfObject::Dictionary(map) = trailer_dict { current_trailer = map; }
                    }
                    break;
                }
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() == 2 {
                    if let (Ok(start_obj), Ok(count)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                        for i in 0..count {
                            let mut entry = String::new();
                            Self::read_line(&mut self.file, &mut entry)?;
                            let entry_parts: Vec<&str> = entry.split_whitespace().collect();
                            if entry_parts.len() == 3 && entry_parts[2] == "n" {
                                let obj_offset: u64 = entry_parts[0].parse().unwrap();
                                self.xref.entry(start_obj + i).or_insert(XrefEntry::Normal(obj_offset));
                            }
                        }
                    }
                } else if line.trim().is_empty() { continue; } else { break; }
            }
        } else {
            self.file.seek(SeekFrom::Start(offset))?;
            Self::skip_object_header(&mut self.file)?;
            let mut clone = self.file.try_clone()?;
            let obj = self.resolve_maybe_stream(&mut clone)?;
            if let PdfObject::Stream(dict, data) = obj {
                if let Some(PdfObject::Name(name)) = dict.get("Type") {
                    if name == "XRef" {
                        current_trailer = dict.clone();
                        self.parse_xref_stream_data(&dict, &data)?;
                    }
                }
            }
        }
        for (k, v) in current_trailer.iter() { self.trailer.entry(k.clone()).or_insert(v.clone()); }
        let prev_offset = if let Some(prev_obj) = current_trailer.get("Prev") {
             match prev_obj {
                 PdfObject::Integer(i) => Some(*i as u64),
                 PdfObject::Real(r) => Some(*r as u64),
                 _ => None,
             }
        } else { None };
        if let Some(off) = prev_offset { self.parse_xref(off)?; }
        Ok(())
    }

    fn parse_xref_stream_data(&mut self, dict: &HashMap<String, PdfObject>, data: &[u8]) -> io::Result<()> {
        let w = if let Some(PdfObject::Array(arr)) = dict.get("W") {
            arr.iter().map(|o| if let PdfObject::Integer(i) = o { *i as usize } else { 0 }).collect::<Vec<usize>>()
        } else { return Err(io::Error::new(io::ErrorKind::InvalidData, "Missing /W in XRef stream")); };
        let columns: usize = if let Some(PdfObject::Dictionary(params)) = dict.get("DecodeParms") {
            if let Some(PdfObject::Integer(c)) = params.get("Columns") { *c as usize } else { w.iter().sum() }
        } else { w.iter().sum() };
        let predictor: i64 = if let Some(PdfObject::Dictionary(params)) = dict.get("DecodeParms") {
            if let Some(PdfObject::Integer(p)) = params.get("Predictor") { *p } else { 1 }
        } else { 1 };
        let unfiltered_data = if predictor >= 10 { Self::unfilter_png(data, columns)? } else { data.to_vec() };
        let size = if let Some(PdfObject::Integer(s)) = dict.get("Size") { *s as u32 } else { 0 };
        let index = if let Some(PdfObject::Array(arr)) = dict.get("Index") {
             arr.iter().map(|o| if let PdfObject::Integer(i) = o { *i as u32 } else { 0 }).collect::<Vec<u32>>()
        } else { vec![0, size] };
        let entry_size: usize = w.iter().sum();
        let mut offset = 0;
        for chunk in index.chunks(2) {
            let start_obj = chunk[0];
            let count = chunk[1];
            for i in 0..count {
                if offset + entry_size > unfiltered_data.len() { break; }
                let entry = &unfiltered_data[offset..offset + entry_size];
                let mut field_offset = 0;
                let mut v = Vec::new();
                for &width in &w {
                    let mut val: u64 = 0;
                    for _ in 0..width { val = (val << 8) | (entry[field_offset] as u64); field_offset += 1; }
                    v.push(val);
                }
                let type_field = if w.len() > 0 && w[0] > 0 { v[0] } else { 1 };
                if type_field == 1 { self.xref.entry(start_obj + i).or_insert(XrefEntry::Normal(v[1])); }
                else if type_field == 2 { self.xref.entry(start_obj + i).or_insert(XrefEntry::Compressed(v[1] as u32, v[2] as u32)); }
                offset += entry_size;
            }
        }
        Ok(())
    }

    pub fn resolve_reference(&mut self, ref_obj: &PdfObject) -> io::Result<PdfObject> {
        if let PdfObject::Reference(obj_num, _) = ref_obj {
            if let Some(cached) = self.resolved_cache.get(obj_num) {
                return Ok(cached.clone());
            }
            let entry = self.xref.get(obj_num).cloned().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("Object {} not found in XREF", obj_num))
            })?;
            let resolved = match entry {
                XrefEntry::Normal(offset) => {
                    let mut reader = self.file.try_clone()?;
                    reader.seek(SeekFrom::Start(offset))?;
                    Self::skip_object_header(&mut reader)?;
                    self.resolve_maybe_stream(&mut reader)?
                }
                XrefEntry::Compressed(container_id, index) => {
                    self.resolve_compressed_object(container_id, index)?
                }
            };
            self.resolved_cache.insert(*obj_num, resolved.clone());
            Ok(resolved)
        } else { Ok(ref_obj.clone()) }
    }

    fn resolve_maybe_stream<R: Read + Seek>(&mut self, reader: &mut R) -> io::Result<PdfObject> {
        let obj = Self::parse_object(reader)?;
        Self::skip_whitespace(reader)?;
        let pos = reader.stream_position()?;
        let mut buf = [0; 6];
        if reader.read(&mut buf).is_ok() && &buf == b"stream" {
            let mut b = [0; 1];
            reader.read_exact(&mut b)?;
            if b[0] == b'\r' { reader.read_exact(&mut b)?; }
            if b[0] != b'\n' { reader.seek(SeekFrom::Current(-1))?; }
            if let PdfObject::Dictionary(ref dict) = obj {
                let length_obj = dict.get("Length").ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Stream missing Length"))?;
                let length_val = self.resolve_reference(&length_obj.clone())?;
                let length = if let PdfObject::Integer(l) = length_val { l as usize } else { 0 };
                let mut stream_data = vec![0; length];
                reader.read_exact(&mut stream_data)?;
                let mut final_data = stream_data;
                if let Some(PdfObject::Name(filter)) = dict.get("Filter") {
                    if filter == "FlateDecode" {
                        let mut decoder = ZlibDecoder::new(&final_data[..]);
                        let mut decoded = Vec::new();
                        let _ = decoder.read_to_end(&mut decoded);
                        final_data = decoded;
                    }
                }
                return Ok(PdfObject::Stream(dict.clone(), final_data));
            }
        }
        reader.seek(SeekFrom::Start(pos))?;
        Ok(obj)
    }

    fn resolve_compressed_object(&mut self, container_id: u32, target_index: u32) -> io::Result<PdfObject> {
        let container = self.resolve_reference(&PdfObject::Reference(container_id, 0))?;
        if let PdfObject::Stream(dict, data) = container {
            let first = if let Some(PdfObject::Integer(f)) = dict.get("First") { *f as usize } else { 0 };
            let mut cursor = Cursor::new(&data);
            let mut offset = 0;
            for _ in 0..=target_index {
                Self::read_token_from_cursor(&mut cursor)?;
                let offset_str = Self::read_token_from_cursor(&mut cursor)?;
                offset = offset_str.parse::<usize>().unwrap_or(0);
            }
            cursor.set_position((first + offset) as u64);
            Self::parse_object(&mut cursor)
        } else { Err(io::Error::new(io::ErrorKind::InvalidData, "Expected Object Stream")) }
    }

    pub fn get_page_count(&mut self) -> io::Result<i64> {
        let root_ref = self.get_root().cloned().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Root not found"))?;
        let root = self.resolve_reference(&root_ref)?;
        if let PdfObject::Dictionary(root_dict) = root {
            let pages_ref = root_dict.get("Pages").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Pages not found"))?;
            let pages = self.resolve_reference(pages_ref)?;
            if let PdfObject::Dictionary(pages_dict) = pages {
                if let Some(PdfObject::Integer(count)) = pages_dict.get("Count") { return Ok(*count); }
            }
        }
        Err(io::Error::new(io::ErrorKind::InvalidData, "Could not find page count"))
    }

    pub fn get_author(&mut self) -> io::Result<String> {
        if let Some(info_ref) = self.get_info().cloned() {
            let info = self.resolve_reference(&info_ref)?;
            if let PdfObject::Dictionary(info_dict) = info {
                if let Some(author_obj) = info_dict.get("Author") {
                    let author_resolved = self.resolve_reference(author_obj)?;
                    if let PdfObject::String(bytes) = author_resolved {
                        if bytes.starts_with(&[0xFE, 0xFF]) {
                            let utf16: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| ((c[0] as u16) << 8) | (c[1] as u16)).collect();
                            return Ok(String::from_utf16_lossy(&utf16));
                        }
                        return Ok(String::from_utf8_lossy(&bytes).into_owned());
                    }
                }
            }
        }
        Ok("Unknown".to_string())
    }

    pub fn get_page_rect(&mut self, page_index: usize) -> io::Result<PageRect> {
        let root_ref = self.get_root().cloned().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Root not found"))?;
        let root = self.resolve_reference(&root_ref)?;
        if let PdfObject::Dictionary(root_dict) = root {
            let pages_ref = root_dict.get("Pages").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Pages not found"))?;
            let page_obj_ref = self.find_page_in_tree(pages_ref, page_index)?;
            let page_obj = self.resolve_reference(&page_obj_ref)?;
            if let PdfObject::Dictionary(page_dict) = page_obj {
                let mut crop_box = None;
                let mut media_box = None;
                
                let mut current_node = Some(page_dict);
                while let Some(node) = current_node {
                    if crop_box.is_none() {
                        if let Some(obj) = node.get("CropBox") {
                            if let Ok(PdfObject::Array(arr)) = self.resolve_reference(obj) {
                                if arr.len() == 4 {
                                    crop_box = Some(arr);
                                }
                            }
                        }
                    }
                    if media_box.is_none() {
                        if let Some(obj) = node.get("MediaBox") {
                            if let Ok(PdfObject::Array(arr)) = self.resolve_reference(obj) {
                                if arr.len() == 4 {
                                    media_box = Some(arr);
                                }
                            }
                        }
                    }
                    if crop_box.is_some() && media_box.is_some() {
                        break;
                    }
                    if let Some(parent_ref) = node.get("Parent") {
                        if let Ok(PdfObject::Dictionary(parent_dict)) = self.resolve_reference(parent_ref) {
                            current_node = Some(parent_dict);
                            continue;
                        }
                    }
                    break;
                }

                let target_box = crop_box.or(media_box);
                if let Some(arr) = target_box {
                    let x1 = match &arr[0] { PdfObject::Integer(i) => *i as f32, PdfObject::Real(r) => *r as f32, _ => 0.0 };
                    let y1 = match &arr[1] { PdfObject::Integer(i) => *i as f32, PdfObject::Real(r) => *r as f32, _ => 0.0 };
                    let x2 = match &arr[2] { PdfObject::Integer(i) => *i as f32, PdfObject::Real(r) => *r as f32, _ => 595.0 };
                    let y2 = match &arr[3] { PdfObject::Integer(i) => *i as f32, PdfObject::Real(r) => *r as f32, _ => 842.0 };
                    
                    let min_x = x1.min(x2);
                    let min_y = y1.min(y2);
                    let width = (x2 - x1).abs();
                    let height = (y2 - y1).abs();
                    return Ok(PageRect { x: min_x, y: min_y, width, height });
                }
            }
        }
        Ok(PageRect { x: 0.0, y: 0.0, width: 595.0, height: 842.0 })
    }

    pub fn get_page_content(&mut self, page_index: usize) -> io::Result<Vec<u8>> {
        let root_ref = self.get_root().cloned().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Root not found"))?;
        let root = self.resolve_reference(&root_ref)?;
        if let PdfObject::Dictionary(root_dict) = root {
            let pages_ref = root_dict.get("Pages").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Pages not found"))?;
            let page_obj_ref = self.find_page_in_tree(pages_ref, page_index)?;
            let page_obj = self.resolve_reference(&page_obj_ref)?;
            if let PdfObject::Dictionary(page_dict) = page_obj {
                if let Some(contents_ref) = page_dict.get("Contents") {
                    let contents = self.resolve_reference(contents_ref)?;
                    let mut all_contents = Vec::new();
                    match contents {
                        PdfObject::Stream(_, data) => { all_contents = data; }
                        PdfObject::Array(arr) => {
                            for stream_ref in arr {
                                let stream = self.resolve_reference(&stream_ref)?;
                                if let PdfObject::Stream(_, data) = stream { all_contents.extend(data); }
                            }
                        }
                        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid Contents type")),
                    }
                    return Ok(all_contents);
                }
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, format!("Page {} not found", page_index)))
    }

    fn find_page_in_tree(&mut self, node_ref: &PdfObject, mut target_index: usize) -> io::Result<PdfObject> {
        let node = self.resolve_reference(node_ref)?;
        if let PdfObject::Dictionary(dict) = node {
            let type_name = dict.get("Type");
            match type_name {
                Some(PdfObject::Name(n)) if n == "Pages" => {
                    let kids = dict.get("Kids").ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Pages missing Kids"))?;
                    if let PdfObject::Array(arr) = kids {
                        for kid_ref in arr {
                            let kid = self.resolve_reference(kid_ref)?;
                            if let PdfObject::Dictionary(kid_dict) = kid {
                                if let Some(PdfObject::Name(n)) = kid_dict.get("Type") {
                                    if n == "Page" {
                                        if target_index == 0 { return Ok(kid_ref.clone()); }
                                        target_index -= 1;
                                    } else if n == "Pages" {
                                        let count = if let Some(PdfObject::Integer(c)) = kid_dict.get("Count") { *c as usize } else { 0 };
                                        if target_index < count { return self.find_page_in_tree(kid_ref, target_index); }
                                        target_index -= count;
                                    }
                                }
                            }
                        }
                    }
                }
                Some(PdfObject::Name(n)) if n == "Page" => { return Ok(node_ref.clone()); }
                _ => {}
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, "Page not found in tree"))
    }

    pub fn parse_cmap(&mut self, cmap_ref: &PdfObject) -> io::Result<HashMap<u32, char>> {
        let mut map = HashMap::new();
        let cmap_obj = self.resolve_reference(cmap_ref)?;
        if let PdfObject::Stream(_, data) = cmap_obj {
            let content = String::from_utf8_lossy(&data);
            let tokens: Vec<&str> = content.split_whitespace().collect();
            let mut i = 0;
            while i < tokens.len() {
                match tokens[i] {
                    "beginbfchar" => {
                        let count = tokens[i-1].parse::<usize>().unwrap_or(0);
                        i += 1;
                        for _ in 0..count {
                            if i + 1 < tokens.len() {
                                let src = tokens[i].trim_matches(|c| c == '<' || c == '>');
                                let dst = tokens[i+1].trim_matches(|c| c == '<' || c == '>');
                                if let (Ok(s), Ok(d)) = (u32::from_str_radix(src, 16), u32::from_str_radix(dst, 16)) {
                                    if let Some(c) = std::char::from_u32(d) { map.insert(s, c); }
                                }
                                i += 2;
                            }
                        }
                    }
                    "beginbfrange" => {
                        let count = tokens[i-1].parse::<usize>().unwrap_or(0);
                        i += 1;
                        for _ in 0..count {
                            if i + 2 < tokens.len() {
                                let start_src = tokens[i].trim_matches(|c| c == '<' || c == '>');
                                let end_src = tokens[i+1].trim_matches(|c| c == '<' || c == '>');
                                let dst = tokens[i+2].trim_matches(|c| c == '<' || c == '>');
                                if let (Ok(ss), Ok(es), Ok(d)) = (u32::from_str_radix(start_src, 16), u32::from_str_radix(end_src, 16), u32::from_str_radix(dst, 16)) {
                                    for offset in 0..=(es - ss) {
                                        if let Some(c) = std::char::from_u32(d + offset) { map.insert(ss + offset, c); }
                                    }
                                }
                                i += 3;
                            }
                        }
                    }
                    _ => i += 1,
                }
            }
        }
        Ok(map)
    }

    pub fn get_root(&self) -> Option<&PdfObject> { self.trailer.get("Root") }
    pub fn get_info(&self) -> Option<&PdfObject> { self.trailer.get("Info") }

    pub fn find_fonts(&mut self) -> Vec<(u32, HashMap<String, PdfObject>)> {
        let mut fonts = Vec::new();
        let obj_nums: Vec<u32> = self.xref.keys().cloned().collect();
        for id in obj_nums {
            if let Ok(obj) = self.resolve_reference(&PdfObject::Reference(id, 0)) {
                let mut dict = match obj {
                    PdfObject::Dictionary(d) => d,
                    PdfObject::Stream(d, _) => d,
                    _ => continue,
                };

                if let Some(PdfObject::Name(t)) = dict.get("Type") {
                    if t == "Font" {
                        // Resolve Encoding if it's a reference
                        if let Some(enc_ref) = dict.get("Encoding").cloned() {
                            if let Ok(enc_obj) = self.resolve_reference(&enc_ref) {
                                dict.insert("Encoding".to_string(), enc_obj);
                            }
                        }
                        fonts.push((id, dict));
                    }
                }
            }
        }
        fonts
    }


    fn read_line<R: Read>(reader: &mut R, buffer: &mut String) -> io::Result<usize> {
        let mut b = [0; 1];
        let mut total = 0;
        while reader.read(&mut b)? > 0 {
            total += 1;
            if b[0] == b'\n' { break; }
            if b[0] != b'\r' { buffer.push(b[0] as char); }
        }
        Ok(total)
    }

    fn skip_whitespace<R: Read + Seek>(reader: &mut R) -> io::Result<()> {
        let mut b = [0; 1];
        while reader.read(&mut b)? > 0 {
            if !b[0].is_ascii_whitespace() { reader.seek(SeekFrom::Current(-1))?; break; }
        }
        Ok(())
    }

    fn skip_object_header<R: Read + Seek>(reader: &mut R) -> io::Result<()> {
        let mut s = String::new();
        let mut b = [0; 1];
        while reader.read(&mut b)? > 0 {
            if b[0].is_ascii_whitespace() { if s == "obj" { return Ok(()); } s.clear(); }
            else { s.push(b[0] as char); if s == "obj" {
                let mut next = [0; 1];
                if reader.read(&mut next).is_ok() {
                    if next[0].is_ascii_whitespace() || next[0] == b'<' { reader.seek(SeekFrom::Current(-1))?; return Ok(()); }
                } else { return Ok(()); }
            } }
        }
        Ok(())
    }

    fn parse_object<R: Read + Seek>(reader: &mut R) -> io::Result<PdfObject> {
        Self::skip_whitespace(reader)?;
        let mut b = [0; 1];
        reader.read_exact(&mut b)?;
        match b[0] {
            b'/' => { reader.seek(SeekFrom::Current(-1))?; Ok(PdfObject::Name(Self::parse_name(reader)?)) }
            b't' | b'f' | b'n' => { reader.seek(SeekFrom::Current(-1))?; Self::parse_keyword_or_bool(reader) }
            b'0'..=b'9' | b'-' => { reader.seek(SeekFrom::Current(-1))?; Self::parse_number_or_reference(reader) }
            b'<' => {
                let mut next = [0; 1];
                reader.read_exact(&mut next)?;
                if next[0] == b'<' { Self::parse_dictionary_content(reader) }
                else { Self::parse_hex_string(reader, next[0]) }
            }
            b'(' => Self::parse_literal_string(reader),
            b'[' => Self::parse_array(reader),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unexpected char: {}", b[0] as char))),
        }
    }

    fn parse_name<R: Read + Seek>(reader: &mut R) -> io::Result<String> {
        Self::skip_whitespace(reader)?;
        let mut b = [0; 1];
        reader.read_exact(&mut b)?;
        if b[0] != b'/' { return Err(io::Error::new(io::ErrorKind::InvalidData, "Expected '/' for name")); }
        let mut name = String::new();
        while reader.read(&mut b)? > 0 {
            if b[0].is_ascii_whitespace() || b[0] == b'/' || b[0] == b'<' || b[0] == b'>' || b[0] == b'[' || b[0] == b']' || b[0] == b'(' || b[0] == b')' {
                reader.seek(SeekFrom::Current(-1))?; break;
            }
            name.push(b[0] as char);
        }
        Ok(name)
    }

    fn parse_dictionary_content<R: Read + Seek>(reader: &mut R) -> io::Result<PdfObject> {
        let mut map = HashMap::new();
        loop {
            Self::skip_whitespace(reader)?;
            let mut buf = [0; 2];
            reader.read_exact(&mut buf)?;
            if &buf == b">>" { break; }
            reader.seek(SeekFrom::Current(-2))?;
            let key = Self::parse_name(reader)?;
            let value = Self::parse_object(reader)?;
            map.insert(key, value);
        }
        Ok(PdfObject::Dictionary(map))
    }

    fn parse_keyword_or_bool<R: Read + Seek>(reader: &mut R) -> io::Result<PdfObject> {
        let mut s = String::new();
        let mut b = [0; 1];
        while reader.read(&mut b)? > 0 {
            if b[0].is_ascii_alphabetic() { s.push(b[0] as char); }
            else { reader.seek(SeekFrom::Current(-1))?; break; }
        }
        match s.as_str() { "true" => Ok(PdfObject::Boolean(true)), "false" => Ok(PdfObject::Boolean(false)), "null" => Ok(PdfObject::Null), _ => Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unknown keyword: {}", s))), }
    }

    fn parse_number_or_reference<R: Read + Seek>(reader: &mut R) -> io::Result<PdfObject> {
        let mut s = String::new();
        let mut b = [0; 1];
        while reader.read(&mut b)? > 0 {
            if b[0].is_ascii_digit() || b[0] == b'.' || b[0] == b'-' { s.push(b[0] as char); }
            else { reader.seek(SeekFrom::Current(-1))?; break; }
        }
        Self::skip_whitespace(reader)?;
        let pos = reader.stream_position()?;
        let mut s2 = String::new();
        while reader.read(&mut b)? > 0 {
            if b[0].is_ascii_digit() { s2.push(b[0] as char); }
            else { reader.seek(SeekFrom::Current(-1))?; break; }
        }
        if !s2.is_empty() {
            Self::skip_whitespace(reader)?;
            if reader.read(&mut b)? > 0 && b[0] == b'R' { return Ok(PdfObject::Reference(s.parse().unwrap(), s2.parse().unwrap())); }
        }
        reader.seek(SeekFrom::Start(pos))?;
        if s.contains('.') { Ok(PdfObject::Real(s.parse().unwrap())) } else { Ok(PdfObject::Integer(s.parse().unwrap())) }
    }

    fn parse_hex_string<R: Read + Seek>(reader: &mut R, first_char: u8) -> io::Result<PdfObject> {
        let mut hex = String::new();
        if first_char.is_ascii_hexdigit() { hex.push(first_char as char); }
        let mut b = [0; 1];
        while reader.read(&mut b)? > 0 {
            if b[0] == b'>' { break; }
            if b[0].is_ascii_hexdigit() { hex.push(b[0] as char); }
        }
        if hex.len() % 2 != 0 { hex.push('0'); }
        let mut bytes = Vec::new();
        for i in (0..hex.len()).step_by(2) { if let Ok(byte) = u8::from_str_radix(&hex[i..i+2], 16) { bytes.push(byte); } }
        Ok(PdfObject::String(bytes))
    }

    fn parse_literal_string<R: Read + Seek>(reader: &mut R) -> io::Result<PdfObject> {
        let mut s = Vec::new();
        let mut b = [0; 1];
        let mut depth = 1;
        while reader.read(&mut b)? > 0 {
            match b[0] { b'(' => depth += 1, b')' => { depth -= 1; if depth == 0 { break; } } _ => {} }
            s.push(b[0]);
        }
        Ok(PdfObject::String(s))
    }

    fn parse_array<R: Read + Seek>(reader: &mut R) -> io::Result<PdfObject> {
        let mut array = Vec::new();
        loop {
            Self::skip_whitespace(reader)?;
            let mut b = [0; 1];
            reader.read_exact(&mut b)?;
            if b[0] == b']' { break; }
            reader.seek(SeekFrom::Current(-1))?;
            array.push(Self::parse_object(reader)?);
        }
        Ok(PdfObject::Array(array))
    }

    fn unfilter_png(data: &[u8], columns: usize) -> io::Result<Vec<u8>> {
        let bpp = 1; let row_size = columns + 1; let mut unfiltered = Vec::new(); let mut prev_row: Vec<u8> = vec![0; columns];
        for row in data.chunks_exact(row_size) {
            let filter_type = row[0]; let row_data = &row[1..]; let mut current_row = vec![0; columns];
            for i in 0..columns {
                let left = if i >= bpp { current_row[i - bpp] } else { 0 };
                let up = prev_row[i];
                let up_left = if i >= bpp { prev_row[i - bpp] } else { 0 };
                match filter_type {
                    0 => current_row[i] = row_data[i],
                    1 => current_row[i] = row_data[i].wrapping_add(left),
                    2 => current_row[i] = row_data[i].wrapping_add(up),
                    3 => current_row[i] = row_data[i].wrapping_add(((left as u16 + up as u16) / 2) as u8),
                    4 => {
                        let p = left as i32 + up as i32 - up_left as i32;
                        let pa = (p - left as i32).abs(); let pb = (p - up as i32).abs(); let pc = (p - up_left as i32).abs();
                        let pr = if pa <= pb && pa <= pc { left } else if pb <= pc { up } else { up_left };
                        current_row[i] = row_data[i].wrapping_add(pr);
                    }
                    _ => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unknown PNG filter type: {}", filter_type))),
                }
            }
            unfiltered.extend_from_slice(&current_row); prev_row = current_row;
        }
        Ok(unfiltered)
    }

    fn read_token_from_cursor(cursor: &mut Cursor<&Vec<u8>>) -> io::Result<String> {
        let mut s = String::new();
        let mut b = [0; 1];
        while cursor.read(&mut b)? > 0 { if !b[0].is_ascii_whitespace() { cursor.set_position(cursor.position() - 1); break; } }
        while cursor.read(&mut b)? > 0 { if b[0].is_ascii_whitespace() { break; } s.push(b[0] as char); }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_pages() {
        let pdf_name = if std::path::Path::new("test.pdf").exists() {
            "test.pdf"
        } else {
            "Eric Hobsbawm - Historia del Siglo XX.pdf"
        };
        let mut parser = Parser::new(pdf_name).unwrap();
        parser.parse_metadata().unwrap();
        let count = parser.get_page_count().unwrap();
        println!("TEST PAGE COUNT: {}", count);
        println!("XREF SIZE: {}", parser.xref.len());
        let mut failures = 0;
        for i in 0..count as usize {
            match parser.get_page_rect(i) {
                Ok(rect) => {
                    if i < 15 || i > (count as usize - 5) {
                        println!("Page {}: {:?}", i, rect);
                    }
                }
                Err(e) => {
                    println!("Page {} rect failed: {:?}", i, e);
                    failures += 1;
                }
            }
            match parser.get_page_content(i) {
                Ok(content) => {
                    if i < 15 || i > (count as usize - 5) {
                        println!("Page {} content len: {}", i, content.len());
                    }
                }
                Err(e) => {
                    println!("Page {} content failed: {:?}", i, e);
                    failures += 1;
                }
            }
        }
        assert_eq!(failures, 0, "Failed to resolve some pages!");
    }
}

