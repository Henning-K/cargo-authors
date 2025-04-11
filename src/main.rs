#![warn(clippy::all)]

extern crate cargo;
use cargo::core::compiler::RustcTargetData;
use cargo::core::resolver::{ForceAllTargets, HasDevUnits};
use cargo::core::Workspace;
use cargo::core::{resolver::CliFeatures, shell::Shell};
use cargo::ops;
use cargo::util::{CargoResult, CliError, context::GlobalContext as GCTXT};
use cargo::CliResult;

extern crate cargo_author;
use cargo_author::Author;

extern crate ripemd;
use ripemd::{Digest, Ripemd160};

extern crate serde;
use serde::{Deserialize, Serialize};

extern crate clap;
use clap::Parser;

use std::{
    collections::{BTreeMap, HashSet},
    env,
    path::Path,
};

#[derive(Clone, Debug, Parser)]
#[command(version, about="List all authors of all dependencies of the current crate.", long_about=None)]
struct Flags {
    /// Write machine-readable (JSON) output to stdout.
    #[arg(short, long, action=clap::ArgAction::SetTrue)]
    json: bool,
    
    /// Replace all authors with their respective hashes in the output.
    #[arg(short='a', long, action=clap::ArgAction::SetTrue)]
    hide_authors: bool,
    
    /// Replace all emails with their respective hashes in the output.
    #[arg(short='e', long, action=clap::ArgAction::SetTrue)]
    hide_emails: bool,
    
    /// Replace all crates with their respective hashes in the output.
    #[arg(short='c', long, action=clap::ArgAction::SetTrue)]
    hide_crates: bool,
    
    /// Don't output author and package name of the current crate.
    #[arg(short='i', long, action=clap::ArgAction::SetTrue)]
    ignore_self: bool,
    
    /// Show the output grouped by the crate name.
    #[arg(long, action=clap::ArgAction::SetTrue)]
    by_crate: bool,
    
    /// Optional path
    #[arg(short, long, value_name="path")]
    path: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct AuthorsResult {
    entries: BTreeMap<String, HashSet<String>>,
}

impl AuthorsResult {
    fn new(v: BTreeMap<String, HashSet<String>>) -> Self {
        AuthorsResult { entries: v }
    }
}

#[derive(Clone)]
struct DependencyAccumulator<'a> {
    gctxt: &'a GCTXT,
    flags: Flags,
}

type Aggregate = CargoResult<BTreeMap<String, HashSet<String>>>;

impl<'a> DependencyAccumulator<'a> {
    fn new(c: &'a GCTXT, flags: Flags) -> Self {
        DependencyAccumulator { gctxt: c, flags }
    }

    fn accumulate(&self) -> Aggregate {
        let path = self
            .flags
            .path
            .clone()
            .unwrap_or_else(|| String::from("."));
        let local_root = Path::new(path.as_str()).canonicalize()?;
        let local_root = local_root.as_path();
        let ws_path = local_root.join("Cargo.toml");
        let ws = Workspace::new(&ws_path, self.gctxt)?;

        let self_pkg = ws.current()?.name().as_str();

        // here starts the code ripped from cargo::ops::cargo_output_metadata.rs
        // because the visibility of the result's (ExportInfo) members returned from
        // cargo::ops::metadata_full()/output_metadata() hinders evaluation
        let mut target_data = RustcTargetData::new(&ws, &[])?;
        let deps = ops::resolve_ws_with_opts(
            &ws,
            &mut target_data,
            &[],
            &CliFeatures::new_all(true),
            &[],
            HasDevUnits::Yes,
            ForceAllTargets::Yes,
            false // dry run? no, this is always a "real" run
        )?;
        let package_set = deps.pkg_set;
        // here ends the ripped code

        let mut result: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        for pkg in package_set.packages() {
            let package = pkg.clone();
            let name = if self.flags.hide_crates {
                format!("{:x}", Ripemd160::digest(package.name().as_bytes()))
            } else {
                package.name().to_string()
            };
            if name == self_pkg && self.flags.ignore_self {
                continue;
            }
            let authors = package.authors().iter().map(|e| {
                if self.flags.hide_authors {
                    format!("{:x}", Ripemd160::digest(e.as_bytes()))
                } else if self.flags.hide_emails {
                    let author = Author::new(e);
                    format!(
                        "{}{}{}",
                        author.name.as_deref().unwrap_or_default(),
                        if author.name.is_some() && author.email.is_some() {
                            " "
                        } else {
                            ""
                        },
                        author
                            .email
                            .as_ref()
                            .map(|s| format!("<{:x}>", Ripemd160::digest(s.as_bytes())))
                            .unwrap_or_default()
                    )
                } else {
                    e.clone()
                }
            });
            for auth in authors {
                let crates = result.entry(auth).or_default();
                crates.insert(name.to_string());
            }
        }

        if self.flags.by_crate {
            let mut result_: BTreeMap<String, HashSet<String>> = BTreeMap::new();
            for (k, vs) in result.iter() {
                for v in vs.iter().cloned() {
                    let authors = result_.entry(v).or_default();
                    authors.insert(k.clone());
                }
            }
            result = result_;
        }

        Ok(result)
    }
}

fn real_main(flags: Flags, gctxt: &GCTXT) -> CliResult {
    let aggregate = match DependencyAccumulator::new(gctxt, flags.clone()).accumulate() {
        Err(ref e) => {
            println!("error: {}", e);

            for e in e.chain() {
                println!("caused by: {}", e);
            }

            // Nightly-only.
            // println!("backtrace: {:?}", e.backtrace());

            ::std::process::exit(1);
        }
        Ok(agg) => agg,
    };

    let max_author_len = aggregate
        .keys()
        .map(|e| e.len())
        .max()
        .unwrap_or_default();

    if flags.json {
        let ar = AuthorsResult::new(aggregate);
        let ar_json = serde_json::to_string(&ar).unwrap();
        println!("{}", ar_json);
    } else {
        println!("Authors and their respective crates for this crate:\n");

        for (author, crates) in aggregate {
            let crates = crates.into_iter().collect::<Vec<String>>();
            let crates = crates[..].join(", ");
            println!("{:<N$}: {}", author, crates, N = max_author_len);
        }
    }

    Ok(())
}

fn main() {
    let gctxt = match GCTXT::default() {
        Ok(cfg) => cfg,
        Err(e) => {
            cargo::exit_with_error(e.into(), &mut Shell::new());
        }
    };

    let result = (|| {
        let args: Vec<_> = env::args_os()
            .map(|s| {
                s.into_string().map_err(|s| {
                    CliError::new(anyhow::anyhow!("invalid argument detected: {:?}", s), 1334)
                })
            })
            .collect::<Result<_, CliError>>()?;
        let flags: Flags = Flags::parse_from(args.iter());
        

        real_main(flags, &gctxt)
    })();

    if let Err(e) = result { cargo::exit_with_error(e, &mut gctxt.shell())}
}
