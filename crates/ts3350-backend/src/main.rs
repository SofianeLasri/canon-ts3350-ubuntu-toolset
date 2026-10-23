//! CUPS backend of the "Canon TS3350" virtual printer.
//!
//! CUPS runs it as root (file mode 0700) with: `job-id user title copies options [file]`.
//! The PDF it receives has already been processed by `pdftopdf` (page ranges, n-up, copies).
//!
//! - Single-sided: the PDF is forwarded unchanged to the real queue.
//! - Duplex: the PDF is dropped in the user's spool and the application is woken up
//!   over D-Bus to open the wizard.

use std::fs::{self, Permissions};
use std::io::Read;
use std::os::unix::fs::{PermissionsExt, chown};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, bail};
use nix::unistd::User;
use ts3350_core::cups::{self, PrintOptions};
use ts3350_core::duplex::Binding;
use ts3350_core::spool::{self, JobMeta, SpoolJob};
use ts3350_core::{APP_DBUS_PATH, APP_ID, BACKEND_SCHEME};

/// Exit codes defined by CUPS (`cups/backend.h`).
const CUPS_BACKEND_OK: u8 = 0;
/// Cancels the job without stopping the queue.
const CUPS_BACKEND_CANCEL: u8 = 5;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 1 {
        // Discovery mode (`lpinfo -v`).
        println!(r#"direct {BACKEND_SCHEME} "Canon TS3350" "Canon TS3350 (manual duplex)""#);
        return ExitCode::from(CUPS_BACKEND_OK);
    }
    if !(6..=7).contains(&args.len()) {
        eprintln!("Usage: {} job-id user title copies options [file]", args[0]);
        return ExitCode::from(CUPS_BACKEND_CANCEL);
    }
    match run(&args) {
        Ok(()) => ExitCode::from(CUPS_BACKEND_OK),
        Err(error) => {
            eprintln!("ERROR: {error:#}");
            ExitCode::from(CUPS_BACKEND_CANCEL)
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    let job_id: u32 = args[1].parse().context("invalid job id")?;
    let user = args[2].as_str();
    let title = if args[3].trim().is_empty() { format!("Job {job_id}") } else { args[3].clone() };
    let options = cups::parse_cups_options(&args[5]);

    let mut data = Vec::new();
    match args.get(6) {
        Some(file) => {
            fs::File::open(file).with_context(|| format!("reading {file}"))?.read_to_end(&mut data)?;
        }
        None => {
            std::io::stdin().read_to_end(&mut data).context("reading standard input")?;
        }
    }
    if !data.starts_with(b"%PDF") {
        bail!("the job is not a PDF: check the queue PPD (cupsFilter2)");
    }

    let target = target_queue()?;
    let print_options = PrintOptions::from_cups_options(&options);

    match cups::duplex_from_options(&options) {
        None => {
            eprintln!("INFO: Single-sided printing on {target}");
            let id = cups::submit_data(&target, &data, &title, user, &print_options)?;
            eprintln!("INFO: Forwarded to {target} ({id})");
        }
        Some(binding) => {
            let account = User::from_name(user)
                .context("looking up the user")?
                .with_context(|| format!("unknown local user: {user}"))?;
            let job = store(job_id, &title, binding, &target, print_options, &account, &data)?;
            eprintln!("INFO: Manual duplex: open the Canon TS3350 wizard");
            notify(&job, &account);
        }
    }
    Ok(())
}

/// Real queue: the one in the `ts3350-duplex:/<queue>` URI, otherwise automatic detection.
fn target_queue() -> Result<String> {
    let uri = std::env::var("DEVICE_URI").unwrap_or_default();
    let preferred = uri
        .strip_prefix(BACKEND_SCHEME)
        .and_then(|rest| rest.strip_prefix(':'))
        .map(|rest| rest.trim_matches('/'))
        .filter(|name| !name.is_empty());
    Ok(cups::find_target_queue(preferred)?)
}

fn store(
    job_id: u32,
    title: &str,
    binding: Binding,
    target: &str,
    options: PrintOptions,
    account: &User,
    data: &[u8],
) -> Result<SpoolJob> {
    if account.name.contains('/') || account.name.starts_with('.') {
        bail!("invalid user name: {}", account.name);
    }
    let (uid, gid) = (Some(account.uid.as_raw()), Some(account.gid.as_raw()));

    let root = Path::new(spool::SPOOL_ROOT);
    fs::create_dir_all(root).context("creating the spool")?;
    fs::set_permissions(root, Permissions::from_mode(0o755))?;

    let dir = spool::user_dir(&account.name);
    fs::create_dir_all(&dir)?;
    chown(&dir, uid, gid)?;
    fs::set_permissions(&dir, Permissions::from_mode(0o700))?;

    let pdf = dir.join(format!("{job_id}.pdf"));
    fs::write(&pdf, data).with_context(|| format!("writing {}", pdf.display()))?;
    chown(&pdf, uid, gid)?;
    fs::set_permissions(&pdf, Permissions::from_mode(0o600))?;

    let job = SpoolJob {
        pdf,
        meta: JobMeta {
            job_id,
            title: title.to_owned(),
            user: account.name.clone(),
            binding,
            target_queue: target.to_owned(),
            options,
            received: JobMeta::now(),
        },
    };
    let meta = job.write_meta()?;
    chown(&meta, uid, gid)?;
    fs::set_permissions(&meta, Permissions::from_mode(0o600))?;
    Ok(job)
}

/// Asks the application (D-Bus activated if needed) to open the job.
fn notify(job: &SpoolJob, account: &User) {
    let uid = account.uid.as_raw();
    let bus = format!("/run/user/{uid}/bus");
    if !Path::new(&bus).exists() {
        eprintln!("INFO: {} has no open session; the job waits in the spool", account.name);
        return;
    }
    let uri = format!("file://{}", percent_encode(&job.pdf.to_string_lossy()));
    let result = Command::new("/usr/bin/gdbus")
        .args([
            "call",
            "--session",
            "--timeout",
            "60",
            "--dest",
            APP_ID,
            "--object-path",
            APP_DBUS_PATH,
            "--method",
            "org.freedesktop.Application.Open",
            &format!("['{uri}']"),
            "{}",
        ])
        .env_clear()
        .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={bus}"))
        .env("HOME", &account.dir)
        .env("PATH", "/usr/bin:/bin")
        .uid(uid)
        .gid(account.gid.as_raw())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output();
    match result {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            eprintln!("WARNING: cannot wake the application up: {}", String::from_utf8_lossy(&output.stderr).trim())
        }
        Err(error) => eprintln!("WARNING: cannot run gdbus: {error}"),
    }
}

/// Minimal URI encoding for a path (`/` is kept).
fn percent_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_paths() {
        assert_eq!(
            percent_encode("/var/spool/ts3350-duplex/jean d'arc/12.pdf"),
            "/var/spool/ts3350-duplex/jean%20d%27arc/12.pdf"
        );
    }
}
