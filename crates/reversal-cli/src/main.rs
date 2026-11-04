//! `reversal` — convert film negative scans to positive LinearRaw DNGs.

mod pipeline;
mod ui;

use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand, ValueEnum};
use reversal_core::invert::{self, Adjust, Hints, Settings};

#[derive(Parser)]
#[command(
    name = "reversal",
    version,
    about = "Convert film negative scans into positive raw DNGs"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert one or more raw negatives to positive DNGs.
    Convert(ConvertArgs),
    /// Start the local web UI for a folder of scans.
    Ui {
        /// Folder with raw negatives.
        dir: PathBuf,
        #[arg(long, default_value_t = 8787)]
        port: u16,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum CompressionArg {
    Ljpeg,
    None,
}

#[derive(clap::Args)]
struct ConvertArgs {
    /// Raw files (NEF, CR2, ARW, DNG, …).
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    /// Output file (single input) or folder. Default: next to each input.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Exposure, −100…+100 (±3 stops).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    exposure: f32,
    /// Contrast, −100…+100.
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    contrast: f32,
    /// Temperature, −100 (cooler) … +100 (warmer).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    temperature: f32,
    /// Tint, −100 (greener) … +100 (more magenta).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    tint: f32,
    /// Saturation on top of the built-in boost, −100 (black and white) … +100.
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    saturation: f32,
    /// Fine rotation in degrees, clockwise positive.
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    straighten: f32,
    /// Crop to the detected image frame (default keeps the whole scan).
    #[arg(long)]
    crop_frame: bool,
    /// Mirror left-right (emulsion-up scans).
    #[arg(long)]
    flip: bool,
    /// Clockwise quarter turns (0–3), applied after mirroring.
    #[arg(long, default_value_t = 0)]
    rotate: u8,
    #[arg(long, value_enum, default_value_t = CompressionArg::Ljpeg)]
    compression: CompressionArg,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Convert(args) => convert(args),
        Command::Ui { dir, port } => ui::serve(dir, port),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn convert(args: ConvertArgs) -> Result<(), Box<dyn std::error::Error>> {
    let adjust = Adjust {
        exposure: args.exposure,
        contrast: args.contrast,
        temperature: args.temperature,
        tint: args.tint,
        saturation: args.saturation,
        straighten: args.straighten,
        crop: None,
        flip_h: args.flip,
        rotate: args.rotate % 4,
    };
    let compression = match args.compression {
        CompressionArg::Ljpeg => reversal_core::dng::Compression::LosslessJpeg,
        CompressionArg::None => reversal_core::dng::Compression::None,
    };
    let outs = pipeline::output_paths(&args.inputs, args.output.as_deref())?;
    for (input, out) in args.inputs.iter().zip(&outs) {
        let t = Instant::now();
        let scan = reversal_core::decode::decode(input)?;
        let mut settings =
            Settings::new(invert::estimate(&scan, &Hints::default()), adjust.clone());
        if args.crop_frame {
            settings.adjust.crop = Some(settings.estimate.frame);
        }
        let bytes = pipeline::export(input, &scan, &settings, compression, out)?;
        println!(
            "{} -> {} ({:.1} MB, {:.1} s)",
            input.display(),
            out.display(),
            bytes as f64 / 1e6,
            t.elapsed().as_secs_f64()
        );
    }
    Ok(())
}
