//! Duplex jobs dropped by the CUPS backend, waiting for the wizard.
//!
//! Layout: `/var/spool/ts3350-duplex/<user>/<job>.pdf` + `<job>.toml`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::cups::PrintOptions;
use crate::duplex::Binding;
use crate::{Error, Result};

pub const SPOOL_ROOT: &str = "/var/spool/ts3350-duplex";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobMeta {
    pub job_id: u32,
    pub title: String,
    pub user: String,
    pub binding: Binding,
    /// Real CUPS queue to print to.
    pub target_queue: String,
    pub options: PrintOptions,
    /// Unix timestamp of reception.
    pub received: u64,
}

impl JobMeta {
    pub fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpoolJob {
    pub pdf: PathBuf,
    pub meta: JobMeta,
}

impl SpoolJob {
    fn meta_path(pdf: &Path) -> PathBuf {
        pdf.with_extension("toml")
    }

    /// Reloads a job from the path of its PDF.
    pub fn from_pdf(pdf: &Path) -> Result<SpoolJob> {
        let text = fs::read_to_string(SpoolJob::meta_path(pdf))?;
        let meta = toml::from_str(&text).map_err(|e| Error::Config(e.to_string()))?;
        Ok(SpoolJob { pdf: pdf.to_owned(), meta })
    }

    /// Writes the metadata next to the PDF.
    pub fn write_meta(&self) -> Result<PathBuf> {
        let path = SpoolJob::meta_path(&self.pdf);
        let text = toml::to_string(&self.meta).map_err(|e| Error::Config(e.to_string()))?;
        fs::write(&path, text)?;
        Ok(path)
    }

    pub fn remove(&self) -> Result<()> {
        fs::remove_file(&self.pdf)?;
        let _ = fs::remove_file(SpoolJob::meta_path(&self.pdf));
        Ok(())
    }
}

pub fn user_dir(user: &str) -> PathBuf {
    Path::new(SPOOL_ROOT).join(user)
}

/// Is the file part of the spool?
pub fn is_spool_path(path: &Path) -> bool {
    path.starts_with(SPOOL_ROOT)
}

/// Pending jobs for `user`, oldest first.
pub fn list_jobs(user: &str) -> Vec<SpoolJob> {
    list_jobs_in(&user_dir(user))
}

fn list_jobs_in(dir: &Path) -> Vec<SpoolJob> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut jobs: Vec<SpoolJob> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "pdf"))
        .filter_map(|p| SpoolJob::from_pdf(&p).ok())
        .collect();
    jobs.sort_by_key(|j| (j.meta.received, j.meta.job_id));
    jobs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_job_metadata() {
        let dir = std::env::temp_dir().join(format!("ts3350-spool-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("12.pdf");
        fs::write(&pdf, b"%PDF-1.4").unwrap();
        let job = SpoolJob {
            pdf: pdf.clone(),
            meta: JobMeta {
                job_id: 12,
                title: "Report".into(),
                user: "alice".into(),
                binding: Binding::ShortEdge,
                target_queue: "Canon_TS3300_series".into(),
                options: PrintOptions { page_size: Some("A4".into()), ..Default::default() },
                received: 1,
            },
        };
        job.write_meta().unwrap();
        assert_eq!(list_jobs_in(&dir), vec![job.clone()]);
        job.remove().unwrap();
        assert!(list_jobs_in(&dir).is_empty());
        fs::remove_dir_all(&dir).unwrap();
        assert!(is_spool_path(Path::new("/var/spool/ts3350-duplex/alice/1.pdf")));
    }
}
