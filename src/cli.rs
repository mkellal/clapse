use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Path to the build directory containing -ftime-trace JSON files
    pub build_dir: PathBuf,

    /// Enable verbose output for debugging the parser
    #[arg(short, long)]
    pub verbose: bool,
}
