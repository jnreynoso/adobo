use std::collections::HashMap;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum PdfObject {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f64),
    String(Vec<u8>),
    Name(String),
    Array(Vec<PdfObject>),
    Dictionary(HashMap<String, PdfObject>),
    Stream(HashMap<String, PdfObject>, Vec<u8>),
    Reference(u32, u16), // (object number, generation number)
}
