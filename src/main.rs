extern crate cargo;
use cargo::core::shell::Shell;
use cargo::core::Workspace;
use cargo::ops::{self, Packages};
use cargo::util::{CargoResult, CliError, Config};
use cargo::CliResult;

#[macro_use]
extern crate failure;

extern crate regex;
use regex::Regex;

#[macro_use]
extern crate lazy_static;

extern crate ripemd160;
use ripemd160::{Digest, Ripemd160};

extern crate serde;
use serde::{Deserialize, Serialize};

extern crate docopt;
use docopt::Docopt;

use std::{
    collections::{BTreeMap, HashSet},
    env,
    path::Path,
};

const USAGE: &str = r"
List all authors of all dependencies of the current crate.

Usage:
  cargo authors [options] [<path>]
  cargo authors (-h | --help)

Options:
  -h --help             Print this message.
  -j --json             Write machine-readable (JSON) output to stdout.
  -i --ignore-self      Don't output author and package name of the current crate.
  -a --hide-authors     Replace all authors with their respective hashes in the output.
  -e --hide-emails      Replace all emails with their respective hashes in the output.
  -c --hide-crates      Replace all crates with their respective hashes in the output.
     --by-crate         Show the output grouped by the crate name.
";

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
    config: &'a Config,
    flags: Flags,
}

type Aggregate = CargoResult<BTreeMap<String, HashSet<String>>>;

impl<'a> DependencyAccumulator<'a> {
    fn new(c: &'a Config, flags: Flags) -> Self {
        DependencyAccumulator { config: c, flags }
    }

    fn accumulate(&self) -> Aggregate {
        lazy_static! {
            static ref EMAIL_PART: Regex = Regex::new("^(?P<name>.* )<(?P<mail>.*)>$").unwrap();
        }
        let path = self
            .flags
            .arg_path
            .clone()
            .unwrap_or_else(|| String::from("."));
        let local_root = Path::new(path.as_str()).canonicalize()?;
        let local_root = local_root.as_path();
        let ws_path = local_root.join("Cargo.toml");
        let ws = Workspace::new(&ws_path, self.config)?;

        let self_pkg = ws.current()?.name().as_str();

        // here starts the code ripped from cargo::ops::cargo_output_metadata.rs
        // because the visibility of the result's (ExportInfo) members returned from
        // cargo::ops::metadata_full()/output_metadata() hinders evaluation
        let specs = Packages::All.to_package_id_specs(&ws)?;
        let deps = ops::resolve_ws_precisely(&ws, &[], true, false, &specs)?;
        let (package_set, _resolve) = deps;
        // here ends the ripped code

        let mut result: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        for pkg in package_set.get_many(package_set.package_ids())? {
            let package = pkg.clone();
            let name = if self.flags.flag_hide_crates {
                format!("{:x}", Ripemd160::digest(package.name().as_bytes()))
            } else {
                package.name().to_string()
            };
            if name == self_pkg && self.flags.flag_ignore_self {
                continue;
            }
            let authors = package.authors().iter().map(|e| {
                if self.flags.flag_hide_authors {
                    format!("{:x}", Ripemd160::digest(e.as_bytes()))
                } else if self.flags.flag_hide_emails {
                    EMAIL_PART
                        .replace(e, |caps: &regex::Captures| {
                            format!(
                                "{}<{:x}>",
                                &caps["name"],
                                Ripemd160::digest(&caps["mail"].as_bytes())
                            )
                        })
                        .into_owned()
                } else {
                    e.clone()
                }
            });
            for auth in authors {
                let crates = result.entry(auth).or_insert_with(HashSet::new);
                crates.insert(name.to_string());
            }
        }

        if self.flags.flag_by_crate {
            let mut result_: BTreeMap<String, HashSet<String>> = BTreeMap::new();
            for (k, vs) in result.iter() {
                for v in vs.iter().cloned() {
                    let authors = result_.entry(v).or_insert_with(HashSet::new);
                    authors.insert(k.clone());
                }
            }
            result = result_;
        }

        Ok(result)
    }
}

#[derive(Clone, Debug, Deserialize)]
struct Flags {
    flag_json: bool,
    flag_by_crate: bool,
    flag_hide_authors: bool,
    flag_hide_emails: bool,
    flag_hide_crates: bool,
    flag_ignore_self: bool,
    arg_path: Option<String>,
}

fn real_main(flags: Flags, config: &Config) -> CliResult {
    let aggregate = match DependencyAccumulator::new(config, flags.clone()).accumulate() {
        Err(ref e) => {
            println!("error: {}", e);

            for e in e.iter_causes() {
                println!("caused by: {}", e);
            }

            println!("backtrace: {:?}", e.backtrace());

            ::std::process::exit(1);
        }
        Ok(agg) => agg,
    };

    let max_author_len = aggregate
        .keys()
        .map(|e| e.len())
        .max()
        .expect("No authors found.");

    if flags.flag_json {
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
    let config = match Config::default() {
        Ok(cfg) => cfg,
        Err(e) => {
            cargo::exit_with_error(e.into(), &mut Shell::new());
        }
    };

    let result = (|| {
        let args: Vec<_> = env::args_os()
            .map(|s| {
                s.into_string().map_err(|s| {
                    CliError::new(
                        failure::format_err!("invalid argument detected: {:?}", s),
                        1334,
                    )
                })
            })
            .collect::<Result<_, CliError>>()?;
        let rest = &args;

        let flags: Flags = Docopt::new(USAGE)
            .and_then(|d| d.argv(rest.iter()).deserialize())
            .unwrap_or_else(|e| e.exit());

        real_main(flags, &config)
    })();

    match result {
        Err(e) => cargo::exit_with_error(e, &mut *config.shell()),
        Ok(()) => {}
    }
}
