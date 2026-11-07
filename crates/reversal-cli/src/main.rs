// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `reversal` — convert film negative scans to positive LinearRaw DNGs.

use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::{Parser, Subcommand, ValueEnum};
use reversal_cli::{pipeline, ui};
use reversal_core::image::Rgb32;
use reversal_core::invert::{self, Adjust, Film, Hints, Settings};
use reversal_core::render::Look;

#[derive(Parser)]
#[command(
    name = "reversal",
    version = env!("REVERSAL_VERSION_LABEL"),
    long_version = concat!(
        env!("REVERSAL_VERSION_LABEL"),
        "\nCopyright (C) 2026 Jan Ambroziewicz\n",
        "Reversal is under the Mozilla Public License 2.0 <https://mozilla.org/MPL/2.0/>.\n",
        "Source code: <https://github.com/janwiktor/reversal>.\n",
        "There is NO WARRANTY, to the extent permitted by law.\n",
        "Uses LibRaw, Copyright (C) 2008-2025 LibRaw LLC, under the CDDL 1.0\n",
        "<https://opensource.org/license/cddl-1-0>. Its source code is available from\n",
        "<https://www.libraw.org/download>.\n",
        "Includes DNG technology under license by Adobe."
    ),
    about = "Convert film negative scans into positive raw DNGs"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert one or more raw scans to positive DNGs.
    Convert(ConvertArgs),
    /// Start the local web UI for a folder of scans.
    Ui {
        /// Folder of raw scans.
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
    #[value(alias = "vivid")]
    Hard,
}

#[derive(Clone, Copy, ValueEnum)]
enum FilmArg {
    Auto,
    Color,
    Bw,
}

#[derive(clap::Args)]
struct ConvertArgs {
    /// Raw files (NEF, CR2, ARW, DNG, …).
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    /// Output file (single input) or folder. Default: next to each input. An existing file
    /// is only replaced if Reversal wrote it.
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
    /// Whites, −100 (recover clipped highlights) … +100 (clip sooner).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    whites: f32,
    /// Film type: detected from each scan, or color or black and white negative.
    #[arg(long, value_enum, default_value_t = FilmArg::Auto)]
    film: FilmArg,
    /// Profile the DNG opens with; the others stay selectable in a raw editor.
    #[arg(long, value_enum, default_value_t = LookArg::Neutral)]
    profile: LookArg,
    /// Temperature, −100 (cooler) … +100 (warmer). Color film only, like tint and saturation.
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    temperature: f32,
    /// Tint, −100 (greener) … +100 (more magenta).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    tint: f32,
    /// Saturation, −100 (black and white) … +100.
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    saturation: f32,
    /// Straighten, in degrees. Positive turns clockwise.
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    straighten: f32,
    /// Center size, in percent: the centered area where color and exposure are measured.
    #[arg(long, default_value_t = invert::DEFAULT_AREA_PERCENT)]
    area_percent: f32,
    /// Crop to the measured area. The default keeps the entire scan.
    #[arg(long)]
    crop_area: bool,
    /// Treat the inputs as one roll: measure one film base across them and use it on each.
    #[arg(long)]
    roll: bool,
    /// Mirror left-right, for scans made emulsion side up.
    #[arg(long)]
    flip: bool,
    /// Clockwise quarter turns (0–3), applied after mirroring.
    #[arg(long, default_value_t = 0)]
    rotate: u8,
    /// DNG compression: lossless JPEG, or none.
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

/// Decodes one input, naming it in the error, since a run can have many.
fn decode_input(input: &Path) -> Result<Rgb32, String> {
    reversal_core::decode::decode(input).map_err(|e| format!("{}: {e}", input.display()))
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
            LookArg::Hard => Look::Hard,
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
        film: match args.film {
            FilmArg::Auto => None,
            FilmArg::Color => Some(Film::Color),
            FilmArg::Bw => Some(Film::Bw),
        },
        area_percent: Some(args.area_percent),
        ..Hints::default()
    };
    let base_rgb = if args.roll {
        let readings = args
            .inputs
            .iter()
            .map(|input| {
                let scan = decode_input(input)?;
                let small = invert::analysis_image(&scan).straighten(args.straighten);
                Ok(invert::detect_base(&small))
            })
            .collect::<Result<Vec<_>, String>>()?;
        // Frames with no clear film don't get a say, unless none has any.
        let mut bases: Vec<_> = readings.iter().filter(|r| r.found).map(|r| r.rgb).collect();
        if bases.is_empty() {
            eprintln!(
                "warning: no clear film detected on any frame, so the roll's film base is a guess"
            );
            bases = readings.iter().map(|r| r.rgb).collect();
        }
        Some(invert::roll_base(&bases))
    } else {
        None
    };
    for (input, out) in args.inputs.iter().zip(&outs) {
        let t = Instant::now();
        let scan = decode_input(input)?;
        let hints = Hints {
            base_rgb,
            ..area_hints.clone()
        };
        // Measured as it will be seen, the same way the web UI does it.
        let small = invert::analysis_image(&scan).straighten(args.straighten);
        let mut settings = Settings::new(invert::estimate_on(&small, &hints), adjust.clone());
        settings.hints = hints;
        if args.crop_area {
            settings.adjust.crop = Some(settings.estimate.area);
        }
        let bytes = pipeline::export(input, &scan, &settings, compression, out)?;
        let film = match settings.estimate.film {
            Film::Color => "color",
            Film::Bw => "black and white",
        };
        println!(
            "{} -> {} ({film}, {:.1} MB, {:.1} s)",
            input.display(),
            out.display(),
            bytes as f64 / 1e6,
            t.elapsed().as_secs_f64()
        );
    }
    Ok(())
}
