//! Blocking eSCL HTTP client (call it outside of the GTK thread).

use std::thread;
use std::time::{Duration, Instant};

use ureq::Agent;

use super::caps::Capabilities;
use super::settings::ScanSettings;
use super::xml::Element;
use crate::i18n::{i18n, i18n_f};
use crate::{Error, Result};

/// A 600 dpi colour scan of the whole platen exceeds 10 MB.
const MAX_DOCUMENT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScannerState {
    Idle,
    Processing,
    Testing,
    Stopped,
    Down,
    Other(String),
}

impl ScannerState {
    pub fn label(&self) -> String {
        match self {
            ScannerState::Idle => i18n("Ready").into(),
            ScannerState::Processing => i18n("Scan in progress").into(),
            ScannerState::Testing => i18n("Warming up").into(),
            ScannerState::Stopped => i18n("Stopped").into(),
            ScannerState::Down => i18n("Out of order").into(),
            ScannerState::Other(s) => s.clone(),
        }
    }
}

#[derive(Clone)]
pub struct Client {
    base: String,
    agent: Agent,
}

impl Client {
    /// `base` is the service URL, for example `http://192.168.1.38/eSCL`.
    pub fn new(base: &str) -> Client {
        let agent: Agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(180)))
            .build()
            .into();
        Client { base: base.trim_end_matches('/').to_owned(), agent }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base, path.trim_start_matches('/'))
    }

    fn get_text(&self, path: &str) -> Result<String> {
        let mut response = self.agent.get(self.url(path)).call()?;
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_string()?;
        if status != 200 {
            return Err(Error::Scanner { status, message: first_line(&body) });
        }
        Ok(body)
    }

    pub fn capabilities(&self) -> Result<Capabilities> {
        Capabilities::parse(&self.get_text("ScannerCapabilities")?)
    }

    pub fn status(&self) -> Result<ScannerState> {
        let root = Element::parse(&self.get_text("ScannerStatus")?)?;
        Ok(match root.child_text("State").unwrap_or("") {
            "Idle" => ScannerState::Idle,
            "Processing" => ScannerState::Processing,
            "Testing" => ScannerState::Testing,
            "Stopped" => ScannerState::Stopped,
            "Down" => ScannerState::Down,
            other => ScannerState::Other(other.to_owned()),
        })
    }

    /// Runs a scan and returns the resulting JPEG.
    pub fn scan(&self, settings: &ScanSettings, version: &str) -> Result<Vec<u8>> {
        self.wait_idle(Duration::from_secs(30))?;

        let body = settings.to_xml(version);
        let mut response =
            self.agent.post(self.url("ScanJobs")).header("Content-Type", "text/xml").send(body.as_str())?;
        let status = response.status().as_u16();
        if status != 201 {
            let text = response.body_mut().read_to_string().unwrap_or_default();
            return Err(Error::Scanner {
                status,
                message: if text.is_empty() { i18n("scan job refused").into() } else { first_line(&text) },
            });
        }
        let location = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Error::Scanner { status, message: i18n("missing Location header").into() })?
            .to_owned();
        let job_url = self.absolute(&location);

        let result = self.fetch_document(&job_url);
        // Release the job on the scanner; some implementations refuse, which is harmless.
        let _ = self.agent.delete(&job_url).call();
        result
    }

    fn absolute(&self, location: &str) -> String {
        if location.starts_with("http://") || location.starts_with("https://") {
            return location.trim_end_matches('/').to_owned();
        }
        // Absolute path (`/eSCL/ScanJobs/…`): keep only the scheme and host.
        let origin_end = self
            .base
            .find("://")
            .and_then(|i| self.base[i + 3..].find('/').map(|j| i + 3 + j))
            .unwrap_or(self.base.len());
        format!("{}{}", &self.base[..origin_end], location.trim_end_matches('/'))
    }

    fn fetch_document(&self, job_url: &str) -> Result<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(240);
        let url = format!("{job_url}/NextDocument");
        loop {
            let mut response = self.agent.get(&url).call()?;
            match response.status().as_u16() {
                200 => {
                    return Ok(response.body_mut().with_config().limit(MAX_DOCUMENT_BYTES).read_to_vec()?);
                }
                // The scanner is warming up or not done yet: try again.
                503 | 409 if Instant::now() < deadline => thread::sleep(Duration::from_millis(800)),
                status => {
                    let text = response.body_mut().read_to_string().unwrap_or_default();
                    return Err(Error::Scanner {
                        status,
                        message: if status == 404 { i18n("no document produced").into() } else { first_line(&text) },
                    });
                }
            }
        }
    }

    fn wait_idle(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.status()? {
                ScannerState::Idle => return Ok(()),
                ScannerState::Processing | ScannerState::Testing if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(500))
                }
                state => {
                    return Err(Error::Scanner {
                        status: 503,
                        message: i18n_f("scanner unavailable ({state})", &[("state", &state.label())]),
                    });
                }
            }
        }
    }
}

fn first_line(text: &str) -> String {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_job_locations() {
        let client = Client::new("http://192.168.1.38:80/eSCL/");
        assert_eq!(client.base_url(), "http://192.168.1.38:80/eSCL");
        assert_eq!(client.absolute("/eSCL/ScanJobs/abc/"), "http://192.168.1.38:80/eSCL/ScanJobs/abc");
        assert_eq!(client.absolute("http://192.168.1.38/eSCL/ScanJobs/abc"), "http://192.168.1.38/eSCL/ScanJobs/abc");
    }
}
