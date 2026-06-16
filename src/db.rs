use redb::{Database, TableDefinition, ReadableDatabase, ReadableTable};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ReadingProgress {
    pub title: String,
    pub author: String,
    pub current_page: u32,
    pub total_pages: u32,
}

impl ReadingProgress {
    pub fn percentage(&self) -> f32 {
        if self.total_pages == 0 {
            0.0
        } else {
            (self.current_page as f32 / self.total_pages as f32) * 100.0
        }
    }
}

const READINGS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("readings");

fn db_path() -> PathBuf {
    let mut path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push(".adobo_readings.redb");
    path
}

pub fn save_progress(file_path: &str, progress: &ReadingProgress) -> Result<(), Box<dyn std::error::Error>> {
    let db = Database::create(db_path())?;
    let write_txn = db.begin_write()?;
    {
        let mut table = write_txn.open_table(READINGS_TABLE)?;
        let json = serde_json::to_string(progress)?;
        table.insert(file_path, json.as_str())?;
    }
    write_txn.commit()?;
    Ok(())
}

pub fn get_progress(file_path: &str) -> Option<ReadingProgress> {
    let db = Database::create(db_path()).ok()?;
    let read_txn = db.begin_read().ok()?;
    let table = read_txn.open_table(READINGS_TABLE).ok()?;
    let val = table.get(file_path).ok()??;
    serde_json::from_str(val.value()).ok()
}

pub fn get_all_readings() -> Vec<(String, ReadingProgress)> {
    let mut results = Vec::new();
    if let Ok(db) = Database::create(db_path()) {
        if let Ok(read_txn) = db.begin_read() {
            if let Ok(table) = read_txn.open_table(READINGS_TABLE) {
                if let Ok(iter) = table.iter() {
                    for result in iter {
                        if let Ok((k, v)) = result {
                            if let Ok(progress) = serde_json::from_str::<ReadingProgress>(v.value()) {
                                results.push((k.value().to_string(), progress));
                            }
                        }
                    }
                }
            }
        }
    }
    results
}
