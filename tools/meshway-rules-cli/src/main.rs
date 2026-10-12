//! `meshway-rules`: publishing-pipeline entry point. File I/O and argument
//! handling only; conversion, artifact checks, catalog assembly, signing and
//! verification are `meshway_rules`. Results are JSON on stdout, logs on stderr.
#![forbid(unsafe_code)]

use clap::{Parser, Subcommand};
use meshway_rules::{
    DomainHit, Name, RuleSet,
    catalog::{self, Catalog, ConversionRecord, PublicKey, SetRecord},
    convert::{self, Format},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write as _,
    net::IpAddr,
    path::{Path, PathBuf},
    process::ExitCode,
};

type Result<T> = std::result::Result<T, String>;

#[derive(Parser)]
#[command(name = "meshway-rules", version, about)]
struct Cli {
    #[command(flatten)]
    logging: meshway_logging::Options,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert a source to an MWRS artifact; prints the conversion record.
    Convert {
        // `format` is the logging argument's id.
        #[arg(long = "format", value_parser = parse_format)]
        source_format: Format,
        input: PathBuf,
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
    },
    /// Header, sections, counts and digest of an artifact.
    Inspect { file: PathBuf },
    /// Match one DNS name or IP address against an artifact.
    Match { file: PathBuf, query: String },
    /// Assemble and sign `index.json` from `<dir>/sets/*/*.json` and
    /// `<dir>/artifacts/`; the sequence is one past an existing output.
    Index {
        #[arg(long = "sign-key")]
        sign_key: PathBuf,
        dir: PathBuf,
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
    },
    /// Verify `index.json` + `index.json.sig` and every referenced artifact.
    Verify {
        #[arg(long = "public-key")]
        public_key: PublicKey,
        index: PathBuf,
        dir: PathBuf,
    },
}

fn parse_format(text: &str) -> std::result::Result<Format, String> {
    text.parse().map_err(|_| {
        let names: Vec<&str> = Format::ALL.iter().map(|f| f.as_str()).collect();
        format!("expected one of {}", names.join(", "))
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Err(error) = cli.logging.init() {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    match run(cli.command) {
        Ok(output) => {
            println!("{output:#}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(%error, "meshway-rules failed");
            ExitCode::FAILURE
        }
    }
}

fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn open(path: &Path) -> Result<RuleSet> {
    RuleSet::from_vec(read(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

/// Same-directory temporary file and rename, so readers never see a partial file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut file = tempfile::NamedTempFile::new_in(dir).map_err(fail)?;
    file.write_all(bytes).map_err(fail)?;
    file.as_file().sync_all().map_err(fail)?;
    file.persist(path).map_err(|e| fail(e.error))?;
    Ok(())
}

fn json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("serializable output")
}

fn run(command: Command) -> Result<Value> {
    match command {
        Command::Convert {
            source_format: format,
            input,
            output,
        } => {
            let source = read(&input)?;
            let conversion = convert::convert(&source, format).map_err(|e| e.to_string())?;
            write_atomic(&output, conversion.artifact.bytes())?;
            let record = ConversionRecord::new(&source, format, &conversion);
            tracing::info!(
                digest = %record.digest,
                skipped = record.report.summary.skipped(),
                "converted"
            );
            Ok(json(&record))
        }
        Command::Inspect { file } => {
            let set = open(&file)?;
            Ok(json!({
                "digest": set.digest(),
                "bytes": set.as_bytes().len(),
                "major": meshway_rules::MAJOR,
                "minor": set.minor(),
                "entries": set.counts().entries(),
                "counts": set.counts(),
                "sections": set.sections(),
            }))
        }
        Command::Match { file, query } => {
            let set = open(&file)?;
            if let Ok(addr) = query.parse::<IpAddr>() {
                return Ok(json!({ "query": addr, "match": set.match_ip(addr) }));
            }
            let name = Name::parse(&query).map_err(|e| format!("{query}: {e}"))?;
            let hit = set.match_domain(name.as_str());
            let member = hit.map(|h| match h {
                DomainHit::Exact => name.as_str().to_owned(),
                DomainHit::Suffix { len } | DomainHit::Subdomain { len } => {
                    name.as_str()[name.as_str().len() - len..].to_owned()
                }
                DomainHit::Keyword { index } => set.keyword(index).unwrap_or_default().to_owned(),
            });
            Ok(json!({ "query": name.as_str(), "match": hit, "member": member }))
        }
        Command::Index {
            sign_key,
            dir,
            output,
        } => index(&sign_key, &dir, &output),
        Command::Verify {
            public_key,
            index,
            dir,
        } => {
            let bytes = read(&index)?;
            let signature = read(&sig_path(&index))?;
            let catalog =
                catalog::verify(&bytes, &signature, &[public_key]).map_err(|e| e.to_string())?;
            for (name, set) in &catalog.sets {
                let artifact = open(&dir.join(catalog::ARTIFACTS).join(set.digest.file_name()))?;
                set.check(&artifact).map_err(|e| format!("{name}: {e}"))?;
            }
            Ok(json!({
                "sequence": catalog.sequence,
                "sets": catalog.sets.len(),
                "keyId": public_key.to_string(),
                "verified": true,
            }))
        }
    }
}

fn sig_path(index: &Path) -> PathBuf {
    let mut name = index.as_os_str().to_owned();
    name.push(".sig");
    PathBuf::from(name)
}

fn index(sign_key: &Path, dir: &Path, output: &Path) -> Result<Value> {
    let seed =
        String::from_utf8(read(sign_key)?).map_err(|_| "signing key is not text".to_owned())?;
    let key = catalog::signing_key(&seed).map_err(|e| e.to_string())?;
    let previous = match fs::read(output) {
        Ok(bytes) => {
            Some(Catalog::parse(&bytes).map_err(|e| format!("{}: {e}", output.display()))?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", output.display())),
    };
    let mut records = BTreeMap::new();
    let sets = dir.join(catalog::SETS);
    for collection in sorted_entries(&sets)? {
        for file in sorted_entries(&collection)? {
            let (Some(c), Some(stem)) = (
                collection.file_name().and_then(|n| n.to_str()),
                file.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix(".json")),
            ) else {
                return Err(format!("{}: not a set record name", file.display()));
            };
            let record: SetRecord = serde_json::from_slice(&read(&file)?)
                .map_err(|e| format!("{}: {e}", file.display()))?;
            records.insert(format!("{c}/{stem}"), record);
        }
    }
    let artifacts = dir.join(catalog::ARTIFACTS);
    let catalog = Catalog::assemble(previous.as_ref(), records, chrono::Utc::now(), |digest| {
        open(&artifacts.join(digest.file_name()))
    })
    .map_err(|e| e.to_string())?;
    let bytes = catalog.to_bytes();
    let signature = catalog::sign(&bytes, &key);
    write_atomic(output, &bytes)?;
    write_atomic(
        &sig_path(output),
        &serde_json::to_vec_pretty(&signature).expect("signature"),
    )?;
    Ok(json!({
        "sequence": catalog.sequence,
        "sets": catalog.sets.len(),
        "keyId": signature.key_id,
    }))
}

fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    entries.sort();
    Ok(entries)
}
