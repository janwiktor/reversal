//! `reversal` — convert film negative scans to positive LinearRaw DNGs.

mod batch;
mod folder;
mod pipeline;
mod ui;

use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand, ValueEnum};
use reversal_core::invert::{self, Adjust, Hints, Settings};
use reversal_core::render::Look;

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
        /// Port on 127.0.0.1; 0 picks a free one.
        #[arg(long, default_value_t = 8787)]
        port: u16,
        /// Exit when stdin closes. For an app that starts this as its engine.
        #[arg(long, hide = true)]
        exit_with_stdin: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum CompressionArg {
    Ljpeg,
    None,
}

#[derive(Clone, Copy, ValueEnum)]
enum LookArg {
    Neutral,
    Flat,
    Vivid,
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
    /// Blacks, −100 (crush) … +100 (lift).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    blacks: f32,
    /// Shadows, −100…+100.
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    shadows: f32,
    /// Highlights, −100…+100.
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    highlights: f32,
    /// Whites, −100 (roll off, bringing back clipped highlights) … +100 (clip sooner).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    whites: f32,
    /// Profile the DNG opens with; the others stay selectable in a raw editor.
    #[arg(long, value_enum, default_value_t = LookArg::Neutral)]
    profile: LookArg,
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
    /// Size of the centred image area that colour and exposure are measured on, in percent.
    #[arg(long, default_value_t = invert::DEFAULT_AREA_PERCENT)]
    area_percent: f32,
    /// Crop to the image area (default keeps the whole scan).
    #[arg(long)]
    crop_area: bool,
    /// Treat the inputs as one roll: measure one film base across them and use it on each.
    #[arg(long)]
    roll: bool,
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
        Command::Ui {
            dir,
            port,
            exit_with_stdin,
        } => ui::serve(dir, port, exit_with_stdin),
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
        blacks: args.blacks,
        shadows: args.shadows,
        highlights: args.highlights,
        whites: args.whites,
        look: match args.profile {
            LookArg::Neutral => Look::Neutral,
            LookArg::Flat => Look::Flat,
            LookArg::Vivid => Look::Vivid,
        },
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
    let area_hints = Hints {
        area_percent: Some(args.area_percent),
        ..Hints::default()
    };
    let base_rgb = if args.roll {
        let bases = args
            .inputs
            .iter()
            .map(|input| {
                let scan = reversal_core::decode::decode(input)?;
                Ok(invert::estimate(&scan, &area_hints).base_rgb)
            })
            .collect::<Result<Vec<_>, reversal_core::decode::DecodeError>>()?;
        Some(invert::roll_base(&bases))
    } else {
        None
    };
    for (input, out) in args.inputs.iter().zip(&outs) {
        let t = Instant::now();
        let scan = reversal_core::decode::decode(input)?;
        let hints = Hints {
            base_rgb,
            ..area_hints.clone()
        };
        let mut settings = Settings::new(invert::estimate(&scan, &hints), adjust.clone());
        settings.hints = hints;
        if args.crop_area {
            settings.adjust.crop = Some(settings.estimate.area);
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
