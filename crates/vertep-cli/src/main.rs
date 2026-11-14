//! The `vt` binary.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use vertep_cli::{Outcome, SchemaFormat, Surface, commands, wallet, webvh};

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
    /// Create, update, witness, publish and serve did:webvh DIDs.
    Webvh {
        #[command(subcommand)]
        command: WebvhCmd,
    },
    /// Agent wallets: issue, accept, present and verify credentials.
    Wallet {
        #[command(subcommand)]
        command: WalletCmd,
    },
}

#[derive(Subcommand)]
enum WalletCmd {
    /// Create a did:key wallet in a directory; prints its DID.
    Create {
        /// The wallet directory.
        #[arg(long)]
        dir: PathBuf,
    },
    /// Trust an issuer when verifying.
    Trust {
        /// The wallet directory.
        #[arg(long)]
        dir: PathBuf,
        /// The issuer DID.
        issuer: String,
    },
    /// Let a verifier (or `*`) receive claims (the consent policy).
    Allow {
        /// The wallet directory.
        #[arg(long)]
        dir: PathBuf,
        /// The verifier DID, or `*`.
        verifier: String,
        /// Claim names, comma-separated.
        claims: String,
    },
    /// List the stored credentials.
    List {
        /// The wallet directory.
        #[arg(long)]
        dir: PathBuf,
    },
    /// Show a credential's metadata and claims.
    Show {
        /// The wallet directory.
        #[arg(long)]
        dir: PathBuf,
        /// The credential digest.
        digest: String,
    },
    /// Issue a credential to a holder key and write it to a file.
    Issue {
        /// The issuer's wallet directory.
        #[arg(long)]
        dir: PathBuf,
        /// The holder (a did:key).
        #[arg(long)]
        to: String,
        /// The credential type.
        #[arg(long = "type")]
        type_: String,
        /// The claims, a JSON object.
        #[arg(long)]
        claims: String,
        /// sd-jwt, di-jcs or di-rdfc.
        #[arg(long, default_value = "sd-jwt")]
        format: String,
        /// Validity in days.
        #[arg(long)]
        valid_days: Option<i64>,
        /// A status list URL to give the credential an entry in.
        #[arg(long)]
        status_list: Option<String>,
        /// The output file.
        #[arg(long)]
        out: PathBuf,
    },
    /// Serve an OpenID4VCI issuer and print the credential offer URI.
    Offer {
        /// The issuer's wallet directory.
        #[arg(long)]
        dir: PathBuf,
        /// Where to listen.
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: String,
        /// The public base URL.
        #[arg(long)]
        base: Option<String>,
        /// The credential type.
        #[arg(long = "type")]
        type_: String,
        /// The claims, a JSON object.
        #[arg(long)]
        claims: String,
        /// A transaction code the holder must enter.
        #[arg(long)]
        tx_code: Option<String>,
        /// Seconds to wait for the holder.
        #[arg(long, default_value_t = 300)]
        timeout: u64,
    },
    /// Store a credential file, or redeem an openid-credential-offer:// URI.
    Accept {
        /// The holder's wallet directory.
        #[arg(long)]
        dir: PathBuf,
        /// A credential file or an offer URI.
        input: String,
        /// The transaction code of an offer.
        #[arg(long)]
        tx_code: Option<String>,
    },
    /// Write a presentation request.
    Request {
        /// The verifier's wallet directory.
        #[arg(long)]
        dir: PathBuf,
        /// Claim names, comma-separated.
        #[arg(long)]
        claims: String,
        /// Acceptable credential types, comma-separated.
        #[arg(long)]
        types: Option<String>,
        /// Why the verifier asks.
        #[arg(long)]
        purpose: Option<String>,
        /// The output file.
        #[arg(long)]
        out: PathBuf,
    },
    /// Answer a request file or an openid4vp:// request through the consent policy.
    Present {
        /// The holder's wallet directory.
        #[arg(long)]
        dir: PathBuf,
        /// A request file or URI.
        input: String,
        /// The output file (for request files).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Verify a presentation against its request.
    Verify {
        /// The verifier's wallet directory.
        #[arg(long)]
        dir: PathBuf,
        /// The request file.
        request: PathBuf,
        /// The presentation file.
        presentation: PathBuf,
        /// A directory of published status list credentials.
        #[arg(long)]
        status_dir: Option<PathBuf>,
    },
    /// Revoke an issued credential (its status entry).
    Revoke {
        /// The issuer's wallet directory.
        #[arg(long)]
        dir: PathBuf,
        /// The credential digest.
        digest: String,
    },
    /// Run the conformance kit for the built-in protocols.
    Conformance {
        /// Seeds per schedule kind.
        #[arg(long, default_value_t = 8)]
        seeds: u64,
    },
}

#[derive(Subcommand)]
enum WebvhCmd {
    /// Create a DID from a new identity; prints the DID.
    Create {
        /// The domain (with an optional port).
        #[arg(long)]
        domain: String,
        /// The path (`a:b`); the DID's files go to /a/b/ instead of /.well-known/.
        #[arg(long)]
        path: Option<String>,
        /// The key manager directory.
        #[arg(long)]
        keys: PathBuf,
        /// Publish to this site directory when no witness proofs are needed.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Announce the next update key by hash.
        #[arg(long)]
        pre_rotation: bool,
        /// The ttl parameter, in seconds.
        #[arg(long)]
        ttl: Option<u32>,
        /// A service, as TYPE=URL (repeatable).
        #[arg(long = "service")]
        services: Vec<String>,
        /// A witness did:key (repeatable).
        #[arg(long = "witness")]
        witnesses: Vec<String>,
        /// How many witness proofs an entry needs (default: all).
        #[arg(long)]
        threshold: Option<u32>,
    },
    /// Replace services or the whole document.
    Update {
        /// The DID.
        did: String,
        /// The key manager directory.
        #[arg(long)]
        keys: PathBuf,
        /// Publish to this site directory when no witness proofs are needed.
        #[arg(long)]
        out: Option<PathBuf>,
        /// A service to add or replace by type, as TYPE=URL (repeatable).
        #[arg(long = "service")]
        services: Vec<String>,
        /// A new document (JSON).
        #[arg(long)]
        document: Option<PathBuf>,
    },
    /// Rotate the update key (to the announced key with pre-rotation).
    Rotate {
        /// The DID.
        did: String,
        /// The key manager directory.
        #[arg(long)]
        keys: PathBuf,
        /// Publish to this site directory when no witness proofs are needed.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Deactivate the DID.
    Deactivate {
        /// The DID.
        did: String,
        /// The key manager directory.
        #[arg(long)]
        keys: PathBuf,
        /// Publish to this site directory when no witness proofs are needed.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Verify a did.jsonl (with the did-witness.json beside it).
    Verify {
        /// The log.
        log: PathBuf,
    },
    /// Add this key's witness proof for the log's last entry.
    Witness {
        /// The log (did-witness.json is written beside it).
        log: PathBuf,
        /// The key manager directory.
        #[arg(long)]
        keys: PathBuf,
        /// The witness's Ed25519 key name.
        #[arg(long)]
        key: String,
    },
    /// Verify the working log and publish it.
    Publish {
        /// The DID.
        did: String,
        /// The key manager directory.
        #[arg(long)]
        keys: PathBuf,
        /// A site directory.
        #[arg(long, conflicts_with_all = ["git", "command"])]
        out: Option<PathBuf>,
        /// A git working copy to commit in and push from.
        #[arg(long)]
        git: Option<PathBuf>,
        /// The directory inside the git working copy.
        #[arg(long, requires = "git")]
        subdir: Option<PathBuf>,
        /// The branch to push to.
        #[arg(long, requires = "git")]
        branch: Option<String>,
        /// Write to --root, then run this command (after --).
        #[arg(long, num_args = 1.., allow_hyphen_values = true, requires = "root")]
        command: Option<Vec<String>>,
        /// The directory for --command.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Serve a site directory of published DIDs over HTTP.
    Serve {
        /// The site directory.
        #[arg(long)]
        root: PathBuf,
        /// The address to listen on.
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: String,
        /// The domain to serve for hosts without their own directory.
        #[arg(long)]
        domain: Option<String>,
    },
    /// Durable-object DIDs.
    Object {
        #[command(subcommand)]
        command: ObjectCmd,
    },
    /// Custody of durable objects.
    Custody {
        #[command(subcommand)]
        command: CustodyCmd,
    },
}

#[derive(Subcommand)]
enum ObjectCmd {
    /// Create an object DID declaring its custodians, who witness its log.
    Create {
        /// The domain.
        #[arg(long)]
        domain: String,
        /// The path (`objects:ticket-8812`).
        #[arg(long)]
        path: String,
        /// The key manager directory.
        #[arg(long)]
        keys: PathBuf,
        /// A custodian, as ROLE,DID,MULTIKEY (repeatable).
        #[arg(long = "custodian", required = true)]
        custodians: Vec<String>,
        /// The quorum (default: the smallest majority).
        #[arg(long)]
        quorum: Option<usize>,
        /// A service, as TYPE=URL (repeatable).
        #[arg(long = "service")]
        services: Vec<String>,
    },
}

#[derive(Subcommand)]
enum CustodyCmd {
    /// Prepare a custodian-set change: the next log entry and the signed request.
    Change {
        /// The object's DID.
        did: String,
        /// The key manager directory.
        #[arg(long)]
        keys: PathBuf,
        /// A new custodian, as ROLE,DID,MULTIKEY (repeatable).
        #[arg(long = "custodian", required = true)]
        custodians: Vec<String>,
        /// The new quorum (default: the smallest majority).
        #[arg(long)]
        quorum: Option<usize>,
        /// Where to write the signed reconfiguration request.
        #[arg(long)]
        request_out: Option<PathBuf>,
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
        Cmd::Webvh { command } => run_webvh(command),
        Cmd::Wallet { command } => run_wallet(command),
    };
    print!("{}", out.stdout);
    eprint!("{}", out.stderr);
    ExitCode::from(u8::try_from(out.code).unwrap_or(1))
}

fn split(list: &str) -> Vec<String> {
    list.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

fn run_wallet(command: WalletCmd) -> Outcome {
    match command {
        WalletCmd::Create { dir } => wallet::create(&dir),
        WalletCmd::Trust { dir, issuer } => wallet::trust(&dir, &issuer),
        WalletCmd::Allow { dir, verifier, claims } => wallet::allow(&dir, &verifier, &split(&claims)),
        WalletCmd::List { dir } => wallet::list(&dir),
        WalletCmd::Show { dir, digest } => wallet::show(&dir, &digest),
        WalletCmd::Issue { dir, to, type_, claims, format, valid_days, status_list, out } => {
            wallet::issue(&wallet::IssueArgs { dir, to, type_, claims, format, valid_days, status_list, out })
        }
        WalletCmd::Offer { dir, listen, base, type_, claims, tx_code, timeout } => {
            wallet::offer(&wallet::OfferArgs { dir, listen, base, type_, claims, tx_code, timeout })
        }
        WalletCmd::Accept { dir, input, tx_code } => wallet::accept(&dir, &input, tx_code.as_deref()),
        WalletCmd::Request { dir, claims, types, purpose, out } => wallet::request(
            &dir,
            &split(&claims),
            &types.map(|t| split(&t)).unwrap_or_default(),
            purpose.as_deref(),
            &out,
        ),
        WalletCmd::Present { dir, input, out } => wallet::present(&dir, &input, out.as_deref()),
        WalletCmd::Verify { dir, request, presentation, status_dir } => {
            wallet::verify(&dir, &request, &presentation, status_dir.as_deref())
        }
        WalletCmd::Revoke { dir, digest } => wallet::revoke(&dir, &digest),
        WalletCmd::Conformance { seeds } => wallet::conformance(seeds),
    }
}

fn run_webvh(command: WebvhCmd) -> Outcome {
    match command {
        WebvhCmd::Create {
            domain,
            path,
            keys,
            out,
            pre_rotation,
            ttl,
            services,
            witnesses,
            threshold,
        } => webvh::create(&webvh::CreateArgs {
            domain,
            path,
            keys,
            out,
            pre_rotation,
            ttl,
            services,
            witnesses,
            threshold,
        }),
        WebvhCmd::Update {
            did,
            keys,
            out,
            services,
            document,
        } => webvh::update(&webvh::UpdateArgs {
            did,
            keys,
            out,
            services,
            document,
        }),
        WebvhCmd::Rotate { did, keys, out } => webvh::rotate(&did, &keys, out.as_deref()),
        WebvhCmd::Deactivate { did, keys, out } => webvh::deactivate(&did, &keys, out.as_deref()),
        WebvhCmd::Verify { log } => webvh::verify(&log),
        WebvhCmd::Witness { log, keys, key } => webvh::witness(&log, &keys, &key),
        WebvhCmd::Publish {
            did,
            keys,
            out,
            git,
            subdir,
            branch,
            command,
            root,
        } => {
            let target = match (out, git, command, root) {
                (Some(d), None, None, _) => webvh::Target::Dir(d),
                (None, Some(worktree), None, _) => webvh::Target::Git {
                    worktree,
                    subdir,
                    branch,
                },
                (None, None, Some(command), Some(root)) => webvh::Target::Command { root, command },
                _ => {
                    return Outcome {
                        stderr: "publish needs one of --out, --git or --command with --root\n"
                            .into(),
                        code: 2,
                        ..Outcome::default()
                    };
                }
            };
            webvh::publish(&did, &keys, &target)
        }
        WebvhCmd::Serve { root, addr, domain } => webvh::serve(&root, &addr, domain),
        WebvhCmd::Object {
            command:
                ObjectCmd::Create {
                    domain,
                    path,
                    keys,
                    custodians,
                    quorum,
                    services,
                },
        } => webvh::object_create(&webvh::ObjectArgs {
            domain,
            path,
            keys,
            custodians,
            quorum,
            services,
        }),
        WebvhCmd::Custody {
            command:
                CustodyCmd::Change {
                    did,
                    keys,
                    custodians,
                    quorum,
                    request_out,
                },
        } => webvh::custody_change_cmd(&webvh::ChangeArgs {
            did,
            keys,
            custodians,
            quorum,
            request_out,
        }),
    }
}
