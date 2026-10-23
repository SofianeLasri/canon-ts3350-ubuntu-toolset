//! CUPS interaction through the `lp`, `lpstat` and `cancel` tools (cups-client package).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

use crate::duplex::Binding;
use crate::i18n::{i18n, i18n_f};
use crate::{BACKEND_SCHEME, Error, Result, VIRTUAL_QUEUE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    pub name: String,
    pub uri: String,
}

/// Options passed to the real queue, using the keywords of its PPD.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintOptions {
    pub page_size: Option<String>,
    pub color_model: Option<String>,
    pub print_quality: Option<String>,
    pub media_type: Option<String>,
}

impl PrintOptions {
    pub fn to_lp_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        for (key, value) in [
            ("PageSize", &self.page_size),
            ("ColorModel", &self.color_model),
            ("cupsPrintQuality", &self.print_quality),
            ("MediaType", &self.media_type),
        ] {
            if let Some(value) = value {
                args.push("-o".to_owned());
                args.push(format!("{key}={value}"));
            }
        }
        args
    }

    /// Extracts the relevant options from a CUPS option list (`argv[5]` of a backend).
    pub fn from_cups_options(options: &[(String, String)]) -> PrintOptions {
        let get = |key: &str| options.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        PrintOptions {
            page_size: get("PageSize").or_else(|| get("media")),
            color_model: get("ColorModel"),
            print_quality: get("cupsPrintQuality"),
            media_type: get("MediaType"),
        }
    }
}

/// Duplex mode requested in the CUPS options, `None` for single-sided printing.
pub fn duplex_from_options(options: &[(String, String)]) -> Option<Binding> {
    let get = |key: &str| options.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    match get("Duplex") {
        Some("DuplexNoTumble") => return Some(Binding::LongEdge),
        Some("DuplexTumble") => return Some(Binding::ShortEdge),
        Some(_) => return None,
        None => {}
    }
    match get("sides") {
        Some("two-sided-long-edge") => Some(Binding::LongEdge),
        Some("two-sided-short-edge") => Some(Binding::ShortEdge),
        _ => None,
    }
}

/// Parses a CUPS option string (`a=b c='d e' \"f g\"=h nocollate`).
pub fn parse_cups_options(input: &str) -> Vec<(String, String)> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = input.chars();
    let mut in_token = false;
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', _) => {
                if let Some(next) = chars.next() {
                    current.push(next);
                    in_token = true;
                }
            }
            ('\'' | '"', None) => {
                quote = Some(c);
                in_token = true;
            }
            (c, Some(q)) if c == q => quote = None,
            (c, None) if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            (c, _) => {
                current.push(c);
                in_token = true;
            }
        }
    }
    if in_token {
        tokens.push(current);
    }
    tokens
        .into_iter()
        .map(|token| match token.split_once('=') {
            Some((k, v)) => (k.to_owned(), v.to_owned()),
            None => match token.strip_prefix("no") {
                Some(name) if !name.is_empty() => (name.to_owned(), "false".to_owned()),
                _ => (token, "true".to_owned()),
            },
        })
        .collect()
}

/// Dry-run mode: `TS3350_DRY_RUN=<folder>` copies the PDFs into that folder
/// instead of printing them (to test the wizard without paper).
pub fn dry_run_dir() -> Option<PathBuf> {
    std::env::var_os("TS3350_DRY_RUN").filter(|v| !v.is_empty()).map(PathBuf::from)
}

const DRY_RUN_PREFIX: &str = "simulation-";
static DRY_RUN_COUNTER: AtomicU32 = AtomicU32::new(1);

fn dry_run_submit(dir: &Path, file: &Path, title: &str) -> Result<String> {
    std::fs::create_dir_all(dir)?;
    let n = DRY_RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let safe: String = title.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
    std::fs::copy(file, dir.join(format!("{n:02}-{safe}.pdf")))?;
    Ok(format!("{DRY_RUN_PREFIX}{n}"))
}

fn run(program: &str, args: &[&str]) -> Result<Output> {
    Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| Error::Cups(i18n_f("cannot run {program}: {error}", &[("program", &program), ("error", &e)])))
}

fn stdout_of(program: &str, args: &[&str]) -> Result<String> {
    let output = run(program, args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Cups(format!("{program}: {}", stderr.trim())));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn queues() -> Result<Vec<Queue>> {
    let output = run("lpstat", &["-v"])?;
    // `lpstat -v` fails when no queue exists: that is not an error here.
    Ok(parse_device_list(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_device_list(text: &str) -> Vec<Queue> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("device for ")?;
            let (name, uri) = rest.split_once(": ")?;
            Some(Queue { name: name.trim().to_owned(), uri: uri.trim().to_owned() })
        })
        .collect()
}

/// Real TS3350 queue: `preferred` if it exists, otherwise the first one that looks like a TS33xx.
pub fn find_target_queue(preferred: Option<&str>) -> Result<String> {
    select_target(&queues()?, preferred)
        .ok_or_else(|| Error::Cups(i18n("no Canon TS3300/TS3350 printer found in CUPS").into()))
}

fn select_target(queues: &[Queue], preferred: Option<&str>) -> Option<String> {
    let real: Vec<&Queue> =
        queues.iter().filter(|q| q.name != VIRTUAL_QUEUE && !q.uri.starts_with(BACKEND_SCHEME)).collect();
    if let Some(name) = preferred
        && real.iter().any(|q| q.name == name)
    {
        return Some(name.to_owned());
    }
    let looks_like = |q: &&&Queue| {
        let haystack = format!("{} {}", q.name, q.uri).to_ascii_lowercase();
        haystack.contains("ts33") || haystack.contains("ts%2033")
    };
    real.iter().find(looks_like).map(|q| q.name.clone())
}

pub fn virtual_queue_installed() -> bool {
    queues().is_ok_and(|qs| qs.iter().any(|q| q.name == VIRTUAL_QUEUE))
}

/// Sends a file to print and returns the job identifier (`queue-123`).
pub fn submit(queue: &str, file: &Path, title: &str, options: &PrintOptions) -> Result<String> {
    if let Some(dir) = dry_run_dir() {
        return dry_run_submit(&dir, file, title);
    }
    let mut args: Vec<String> = vec!["-d".into(), queue.into(), "-t".into(), title.into()];
    args.extend(options.to_lp_args());
    args.push("--".into());
    args.push(file.to_string_lossy().into_owned());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    parse_request_id(&stdout_of("lp", &refs)?)
}

/// Variant used by the backend: data on standard input, on behalf of `user`.
pub fn submit_data(queue: &str, data: &[u8], title: &str, user: &str, options: &PrintOptions) -> Result<String> {
    let mut args: Vec<String> = vec!["-d".into(), queue.into(), "-t".into(), title.into(), "-U".into(), user.into()];
    args.extend(options.to_lp_args());
    let mut child = Command::new("lp")
        .args(&args)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Cups(i18n_f("cannot run {program}: {error}", &[("program", &"lp"), ("error", &e)])))?;
    child.stdin.take().expect("piped stdin").write_all(data)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(Error::Cups(format!("lp: {}", String::from_utf8_lossy(&output.stderr).trim())));
    }
    parse_request_id(&String::from_utf8_lossy(&output.stdout))
}

fn parse_request_id(stdout: &str) -> Result<String> {
    stdout
        .split_whitespace()
        .skip_while(|w| *w != "is")
        .nth(1)
        .map(str::to_owned)
        .ok_or_else(|| Error::Cups(i18n_f("unexpected answer from lp: {output}", &[("output", &stdout.trim())])))
}

pub fn cancel(job_id: &str) -> Result<()> {
    if job_id.starts_with(DRY_RUN_PREFIX) {
        return Ok(());
    }
    stdout_of("cancel", &[job_id]).map(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobProgress {
    Queued,
    Printing,
    Done,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrinterStatus {
    pub enabled: bool,
    pub printing: Option<String>,
    pub alerts: Vec<String>,
    pub message: Option<String>,
}

impl PrinterStatus {
    /// Message to show while printing when something is wrong.
    pub fn problem(&self) -> Option<String> {
        let known: Vec<&str> = self.alerts.iter().filter_map(|a| alert_label(a)).collect();
        if !known.is_empty() {
            return Some(known.join(" · "));
        }
        if !self.enabled {
            return Some(match &self.message {
                Some(m) => i18n_f("Printer stopped: {message}", &[("message", m)]),
                None => i18n("Printer stopped in CUPS").into(),
            });
        }
        None
    }
}

/// Human-readable text for the most common IPP printer-state-reasons.
pub fn alert_label(alert: &str) -> Option<&'static str> {
    let base = alert.trim_end_matches("-error").trim_end_matches("-warning").trim_end_matches("-report");
    Some(match base {
        "media-empty" | "media-needed" => i18n("No paper in the rear tray"),
        "media-jam" => i18n("Paper jam"),
        "cover-open" | "door-open" => i18n("Cover open"),
        "marker-supply-empty" => i18n("Ink cartridge empty"),
        "marker-supply-low" => i18n("Ink level low"),
        "offline" | "shutdown" => i18n("Printer offline"),
        "connecting-to-device" => i18n("Connecting to the printer…"),
        "input-tray-missing" => i18n("Paper tray missing"),
        "output-area-full" => i18n("Output tray full"),
        _ => return None,
    })
}

pub fn printer_status(queue: &str) -> Result<PrinterStatus> {
    if dry_run_dir().is_some() {
        return Ok(PrinterStatus { enabled: true, ..Default::default() });
    }
    Ok(parse_printer_status(&stdout_of("lpstat", &["-l", "-p", queue])?))
}

fn parse_printer_status(text: &str) -> PrinterStatus {
    let mut status = PrinterStatus { enabled: true, ..Default::default() };
    let mut lines = text.lines();
    if let Some(first) = lines.next() {
        status.enabled = !first.contains(" disabled ");
        if let Some(rest) = first.split(" now printing ").nth(1) {
            status.printing = rest.split_whitespace().next().map(|j| j.trim_end_matches('.').to_owned());
        }
    }
    for line in lines {
        let line = line.trim();
        if let Some(alerts) = line.strip_prefix("Alerts:") {
            status.alerts = alerts.split_whitespace().filter(|a| *a != "none").map(str::to_owned).collect();
        } else if status.message.is_none() && !line.is_empty() && !line.contains(':') && !line.starts_with('(') {
            // Free text below the header: printer-state-message.
            status.message = Some(line.to_owned());
        }
    }
    status
}

pub fn job_progress(queue: &str, job_id: &str) -> Result<JobProgress> {
    if job_id.starts_with(DRY_RUN_PREFIX) {
        return Ok(JobProgress::Done);
    }
    let pending = stdout_of("lpstat", &["-W", "not-completed", "-o", queue])?;
    let queued = pending.lines().any(|l| l.split_whitespace().next() == Some(job_id));
    if !queued {
        return Ok(JobProgress::Done);
    }
    let status = printer_status(queue)?;
    Ok(if status.printing.as_deref() == Some(job_id) { JobProgress::Printing } else { JobProgress::Queued })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_list_and_selects_target() {
        let list = parse_device_list(
            "device for Canon_TS3350: ts3350-duplex:/Canon_TS3300_series\n\
             device for HP_Office: ipp://10.0.0.2/ipp/print\n\
             device for Canon_TS3300_series: implicitclass://Canon_TS3300_series/\n",
        );
        assert_eq!(list.len(), 3);
        assert_eq!(select_target(&list, None).as_deref(), Some("Canon_TS3300_series"));
        assert_eq!(select_target(&list, Some("HP_Office")).as_deref(), Some("HP_Office"));
        assert_eq!(select_target(&list, Some("Missing")).as_deref(), Some("Canon_TS3300_series"));
        assert_eq!(select_target(&list[..2], None), None);
    }

    #[test]
    fn parses_options() {
        let opts = parse_cups_options(
            "Duplex=DuplexTumble PageSize=A4 job-name='My report' noCollate title=\"a b\" esc=x\\ y",
        );
        assert_eq!(duplex_from_options(&opts), Some(Binding::ShortEdge));
        assert!(opts.contains(&("job-name".into(), "My report".into())));
        assert!(opts.contains(&("Collate".into(), "false".into())));
        assert!(opts.contains(&("title".into(), "a b".into())));
        assert!(opts.contains(&("esc".into(), "x y".into())));
        assert_eq!(PrintOptions::from_cups_options(&opts).page_size.as_deref(), Some("A4"));

        assert_eq!(duplex_from_options(&parse_cups_options("Duplex=None sides=two-sided-long-edge")), None);
        assert_eq!(duplex_from_options(&parse_cups_options("sides=two-sided-long-edge")), Some(Binding::LongEdge));
        assert_eq!(duplex_from_options(&[]), None);
    }

    #[test]
    fn parses_request_ids() {
        assert_eq!(
            parse_request_id("request id is Canon_TS3300_series-42 (1 file(s))\n").unwrap(),
            "Canon_TS3300_series-42"
        );
        assert!(parse_request_id("oops").is_err());
    }

    #[test]
    fn parses_printer_status() {
        let status = parse_printer_status(
            "printer Canon_TS3300_series now printing Canon_TS3300_series-7.  enabled since Fri\n\
             \tWaiting for paper\n\
             \tAlerts: media-empty-error\n\
             \tDescription: Canon",
        );
        assert_eq!(status.printing.as_deref(), Some("Canon_TS3300_series-7"));
        assert_eq!(status.message.as_deref(), Some("Waiting for paper"));
        assert_eq!(status.problem().as_deref(), Some(i18n("No paper in the rear tray")));

        let idle = parse_printer_status("printer X is idle.  enabled since Fri\n\tAlerts: none\n");
        assert_eq!(idle.problem(), None);
        let disabled = parse_printer_status("printer X disabled since Fri -\n\treason unknown\n");
        assert!(disabled.problem().is_some());
    }
}
