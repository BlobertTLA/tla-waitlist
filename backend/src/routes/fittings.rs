use crate::data::yamlhelper;
use crate::util::madness::Madness;
use eve_data_core::TypeID;
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct DNAFitting {
    pub name: String,
    pub dna: String,
}
#[derive(Debug, Serialize)]
struct FittingResponse {
    fittingdata: Option<Vec<DNAFitting>>,
    notes: Option<Vec<FittingNote>>,
    implants: Option<Vec<ImplantSet>>,
    rules: Option<Vec<TypeID>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FittingNote {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct NoteFile {
    notes: Vec<FittingNote>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ImplantEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ImplantSlot {
    pub slot: u8,
    #[serde(default)]
    pub default: Vec<ImplantEntry>,
    #[serde(default)]
    pub alternative: Vec<ImplantEntry>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ImplantSet {
    pub name: String,
    pub slots: Vec<ImplantSlot>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ImplantFile {
    sets: Vec<ImplantSet>,
}

fn load_notes_from_file() -> Vec<FittingNote> {
    let file: NoteFile = yamlhelper::from_file("./data/fitnotes.yaml");
    file.notes
}

fn load_implants_from_file() -> Vec<ImplantSet> {
    let file: ImplantFile = yamlhelper::from_file("./data/implants.yaml");
    file.sets
}

use std::fs;
use std::io::Write;
use std::path::Path;

fn save_yaml_file(path: &str, backup_prefix: &str, yaml_content: &str) -> Result<(), Madness> {
    create_backup(path, backup_prefix)?;

    let temp_path = format!("{}.tmp", path);
    let mut temp_file = fs::File::create(&temp_path)
        .map_err(|e| Madness::BadRequest(format!("Failed to create temp file: {}", e)))?;

    temp_file
        .write_all(yaml_content.as_bytes())
        .map_err(|e| Madness::BadRequest(format!("Failed to write temp file: {}", e)))?;

    temp_file
        .sync_all()
        .map_err(|e| Madness::BadRequest(format!("Failed to sync temp file: {}", e)))?;

    fs::rename(&temp_path, path)
        .map_err(|e| Madness::BadRequest(format!("Failed to rename temp file: {}", e)))?;

    Ok(())
}

fn create_backup(source: &str, backup_prefix: &str) -> Result<(), Madness> {
    use std::time::SystemTime;

    if !Path::new(source).exists() {
        return Ok(());
    }

    let timestamp = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let backup_path = format!("{}.backup.{}", source, timestamp);

    fs::copy(source, &backup_path)
        .map_err(|e| Madness::BadRequest(format!("Failed to create backup: {}", e)))?;

    // Keep only last 5 backups
    let mut backups: Vec<_> = fs::read_dir("./data")
        .map_err(|e| Madness::BadRequest(format!("Failed to read data directory: {}", e)))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(backup_prefix) {
                if let Some(timestamp_str) = name.strip_prefix(backup_prefix) {
                    if let Ok(ts) = timestamp_str.parse::<u64>() {
                        return Some((entry.path(), ts));
                    }
                }
            }
            None
        })
        .collect();

    backups.sort_by(|a, b| b.1.cmp(&a.1));

    for (path, _) in backups.into_iter().skip(5) {
        let _ = fs::remove_file(path);
    }

    Ok(())
}

pub mod fitnotes {
    use super::*;

    pub fn save_fitnotes_to_file(yaml_content: &str) -> Result<(), Madness> {
        let _: NoteFile = serde_yaml::from_str(yaml_content)
            .map_err(|e| Madness::BadRequest(format!("Invalid YAML: {}", e)))?;
        save_yaml_file(
            "./data/fitnotes.yaml",
            "fitnotes.yaml.backup.",
            yaml_content,
        )
    }
}

pub mod implants {
    use super::*;

    pub fn save_implants_to_file(yaml_content: &str) -> Result<(), Madness> {
        let _: ImplantFile = serde_yaml::from_str(yaml_content)
            .map_err(|e| Madness::BadRequest(format!("Invalid YAML: {}", e)))?;
        save_yaml_file(
            "./data/implants.yaml",
            "implants.yaml.backup.",
            yaml_content,
        )
    }
}

#[get("/api/fittings")]
async fn fittings() -> Result<Json<FittingResponse>, Madness> {
    let fits_data = crate::data::fits::get_fits();
    let fits_guard = fits_data.read().unwrap();
    let fits = fits_guard
        .values()
        .flatten()
        .filter(|fit| !fit.name.contains("ALTERNATIVE"))
        .map(|fit| DNAFitting {
            name: fit.name.clone(),
            dna: fit.fit.to_dna().unwrap().clone(),
        })
        .collect::<Vec<_>>();

    let mut logirules = Vec::new();

    for rule in crate::data::categories::rules() {
        if rule.1 == "logi" {
            logirules.push(rule.0)
        }
    }
    Ok(Json(FittingResponse {
        fittingdata: Some(fits),
        notes: Some(load_notes_from_file()),
        implants: Some(load_implants_from_file()),
        rules: Some(logirules),
    }))
}

pub fn routes() -> Vec<rocket::Route> {
    routes![fittings]
}
