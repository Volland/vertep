//! The `vt` binary.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use vertep_cli::{Outcome, SchemaFormat, Surface, commands};

/// Tools for Vertep charts in SCXML, YAML and RDF.
#[derive(Parser)]
#[command(name = "vt", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Type-check a chart and report every error with its location.
    Typecheck {
        /// The chart file.
        chart: PathBuf,
    },
    /// Print a projection of the chart's types.
    Schema {
        /// The chart file.
        chart: PathBuf,
        /// The projection.
        #[arg(long, value_enum)]
        format: Format,
    },
    /// Check well-formedness and SHACL conformance.
    Validate {
        /// The chart file.
        chart: PathBuf,
    },
    /// Print the chart version IRI.
    Hash {
        /// The chart file.
        chart: PathBuf,
    },
    /// Convert a chart to another surface, refusing lossy conversions.
    Convert {
        /// The chart file.
        input: PathBuf,
        /// The target surface.
        #[arg(long, value_enum)]
        to: Target,
        /// Write here instead of standard output.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Run the Lean checker and print the certificate.
    Certify {
        /// The chart file.
        chart: PathBuf,
    },
    /// Generate code from a chart.
    Gen {
        #[command(subcommand)]
        command: GenCmd,
    },
    /// Cluster operations.
    Cluster {
        #[command(subcommand)]
        command: ClusterCmd,
    },
}

#[derive(Subcommand)]
enum GenCmd {
    /// The TypeScript module for @vertep/web: events, context, state ids and a typed actor.
    Ts {
        /// The chart file.
        chart: PathBuf,
        /// The prefix of the generated names (default: the chart name or file stem).
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Subcommand)]
enum ClusterCmd {
    /// Double the shard count of a stopped cluster (offline).
    Reshard {
        /// The cluster data directory: `cluster.meta`, the shard files and `leases`.
        dir: PathBuf,
        /// Where to write the resharded files and metadata.
        out: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Rust,
    Jsonschema,
    Shacl,
    Cel,
}

#[derive(Clone, Copy, ValueEnum)]
enum Target {
    Scxml,
    Yaml,
    Turtle,
    Trig,
    Nquads,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let out: Outcome = match cli.command {
        Cmd::Typecheck { chart } => commands::typecheck(&chart),
        Cmd::Schema { chart, format } => commands::schema(
            &chart,
            match format {
                Format::Rust => SchemaFormat::Rust,
                Format::Jsonschema => SchemaFormat::JsonSchema,
                Format::Shacl => SchemaFormat::Shacl,
                Format::Cel => SchemaFormat::Cel,
            },
        ),
        Cmd::Validate { chart } => commands::validate(&chart),
        Cmd::Hash { chart } => commands::hash(&chart),
        Cmd::Convert { input, to, output } => commands::convert(
            &input,
            match to {
                Target::Scxml => Surface::Scxml,
                Target::Yaml => Surface::Yaml,
                Target::Turtle => Surface::Turtle,
                Target::Trig => Surface::TriG,
                Target::Nquads => Surface::NQuads,
            },
            output.as_deref(),
        ),
        Cmd::Certify { chart } => commands::certify(&chart),
        Cmd::Gen {
            command: GenCmd::Ts { chart, name },
        } => commands::gen_ts(&chart, name.as_deref()),
        Cmd::Cluster {
            command: ClusterCmd::Reshard { dir, out },
        } => commands::cluster_reshard(&dir, &out),
    };
    print!("{}", out.stdout);
    eprint!("{}", out.stderr);
    ExitCode::from(u8::try_from(out.code).unwrap_or(1))
}
