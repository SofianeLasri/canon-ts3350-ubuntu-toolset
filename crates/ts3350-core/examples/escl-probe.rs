//! Diagnostic tool: discovers the scanner, prints its capabilities and runs a
//! low-resolution scan.
//!
//! `cargo run -p ts3350-core --example escl-probe -- [url] [file.jpg]`

use std::time::{Duration, Instant};

use ts3350_core::escl::{self, ColorMode, Paper, ScanSettings};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let url = match args.next() {
        Some(url) if !url.is_empty() => url,
        _ => {
            let found = escl::discover(Duration::from_secs(6));
            for s in &found {
                println!("found: {} → {}", s.name, s.url);
            }
            found.into_iter().next().ok_or("no scanner found")?.url
        }
    };
    let output = args.next().unwrap_or_else(|| "escl-probe.jpg".into());

    let client = escl::Client::new(&url);
    let caps = client.capabilities()?;
    println!("{caps:#?}");
    println!("state: {:?}", client.status()?);

    let input = caps.input().ok_or("no scan source")?;
    let settings = ScanSettings {
        color_mode: ColorMode::Rgb24,
        resolution: 75,
        intent: Some("Preview".into()),
        content_type: Some("Photo".into()),
        region: Paper::A4.region(input).unwrap(),
        sharpen: caps.sharpen.map(|r| r.normal),
        compression: caps.compression.map(|r| r.normal),
        background_reduction: caps.background_reduction,
    };
    let start = Instant::now();
    let jpeg = client.scan(&settings, &caps.version)?;
    let (w, h, gray) = ts3350_core::export::jpeg_info(&jpeg)?;
    println!("{} bytes, {w}×{h}, gray={gray}, in {:.1} s", jpeg.len(), start.elapsed().as_secs_f32());
    std::fs::write(&output, jpeg)?;
    println!("written to {output}");
    Ok(())
}
