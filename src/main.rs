use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::io::Read;
use std::path::PathBuf;

use lawgit::{
    amend_law, create_law, init_repository, list_laws, list_rules, prompt_and_create_rule,
    read_law, read_rule, validate_repository, validate_rule_repository, Amendment,
    AmendmentChange, NewLaw,
};

#[derive(Parser)]
#[command(
    name = "lawgit",
    version,
    about = "Manage structured legal documents in a Git repository"
)]
struct Cli {
    /// Path to the law repository.
    #[arg(short, long, global = true, default_value = ".")]
    repo: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a Git repository for legal documents.
    Init,

    /// Register a new legal document and generate its UUID v4.
    New {
        /// Official legal document name.
        #[arg(long)]
        name: String,

        /// Legal document type, such as 法律 or 政令.
        #[arg(long = "type")]
        law_type: String,

        /// Official legal number assigned upon promulgation.
        #[arg(long = "law-number")]
        law_number: Option<String>,

        /// e-Gov Legal ID, when available.
        #[arg(long = "e-gov-id")]
        e_gov_id: Option<String>,

        /// Original promulgation date (YYYY-MM-DD).
        #[arg(long = "promulgated-date")]
        promulgated_date: Option<String>,

        /// Original commencement date (YYYY-MM-DD).
        #[arg(long = "effective-date")]
        effective_date: Option<String>,
    },

    /// Apply one amendment from a complete XML file or unified diff and commit it.
    Amend {
        /// lawgit UUID v4 of the existing legal document.
        #[arg(long)]
        uuid: String,

        /// Complete replacement XML file, or - to read from stdin.
        #[arg(long, conflicts_with = "patch")]
        input: Option<PathBuf>,

        /// Unified diff for laws/<UUID>.xml, or - to read from stdin.
        #[arg(long, conflicts_with = "input")]
        patch: Option<PathBuf>,

        /// Promulgation date of the amending law (YYYY-MM-DD).
        #[arg(long = "promulgated-date")]
        promulgated_date: String,

        /// Commencement date(s) of the amendment. May be supplied more than once.
        #[arg(long = "effective-date", action = clap::ArgAction::Append)]
        effective_dates: Vec<String>,

        /// Git commit message describing the amendment.
        #[arg(long)]
        message: String,
    },

    /// Validate every XML document in the laws directory.
    Validate,

    /// List registered legal documents.
    List,

    /// Print the XML document for a lawgit UUID.
    Show {
        /// lawgit UUID v4.
        uuid: String,
    },

    /// Create and manage user-authored rule documents.
    Rule {
        #[command(subcommand)]
        command: RuleCommands,
    },
}

#[derive(Subcommand)]
enum RuleCommands {
    /// Interactively create a rule document.
    Make,

    /// List all registered rule documents.
    List,

    /// Print a rule document by UUID.
    Show {
        /// Rule UUID v4.
        uuid: String,
    },

    /// Validate every XML document in the rules directory.
    Validate,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn read_payload(path: &PathBuf) -> Result<String> {
    if path == &PathBuf::from("-") {
        let mut content = String::new();
        std::io::stdin()
            .read_to_string(&mut content)
            .context("failed to read input from stdin")?;
        Ok(content)
    } else {
        std::fs::read_to_string(path)
            .with_context(|| format!("cannot read input file: {}", path.display()))
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init => {
            init_repository(&cli.repo)?;
            println!("Initialized law repository: {}", cli.repo.display());
            println!("Legal documents will be stored in: {}", cli.repo.join("laws").display());
        }
        Commands::New {
            name,
            law_type,
            law_number,
            e_gov_id,
            promulgated_date,
            effective_date,
        } => {
            let law = create_law(
                &cli.repo,
                &NewLaw {
                    name,
                    law_type,
                    law_number,
                    e_gov_id,
                    promulgated_date,
                    effective_date,
                },
            )?;
            println!("Registered: {}", law.name);
            println!("UUID:      {}", law.uuid);
            println!("File:      {}", law.path.display());
            println!("Edit the XML body, then run lawgit validate before committing.");
        }
        Commands::Amend {
            uuid,
            input,
            patch,
            promulgated_date,
            effective_dates,
            message,
        } => {
            let change = match (input, patch) {
                (Some(path), None) => AmendmentChange::CompleteXml(read_payload(&path)?),
                (None, Some(path)) => AmendmentChange::UnifiedDiff(read_payload(&path)?),
                _ => anyhow::bail!("specify exactly one of --input or --patch"),
            };
            let commit = amend_law(
                &cli.repo,
                &Amendment {
                    uuid,
                    change,
                    promulgated_date,
                    effective_dates,
                    message,
                },
            )?;
            println!("Amendment committed: {commit}");
        }
        Commands::Validate => {
            let laws = validate_repository(&cli.repo)?;
            println!("Validation passed: {} law document(s).", laws.len());
        }
        Commands::List => {
            let laws = list_laws(&cli.repo)?;
            if laws.is_empty() {
                println!("No law documents are registered.");
            } else {
                println!("{:<36}  {:<8}  NAME", "UUID", "TYPE");
                for law in laws {
                    println!("{:<36}  {:<8}  {}", law.uuid, law.law_type, law.name);
                }
            }
        }
        Commands::Show { uuid } => {
            let (_law, xml) = read_law(&cli.repo, &uuid)?;
            print!("{xml}");
        }
        Commands::Rule { command } => match command {
            RuleCommands::Make => {
                let rule = prompt_and_create_rule(&cli.repo)?;
                println!("Created rule: {}", rule.name);
                println!("UUID:         {}", rule.uuid);
                println!("File:         {}", rule.path.display());
            }
            RuleCommands::List => {
                let rules = list_rules(&cli.repo)?;
                if rules.is_empty() {
                    println!("No rule documents are registered.");
                } else {
                    println!("{:<36}  {:<12}  NAME", "UUID", "TYPE");
                    for rule in rules {
                        println!("{:<36}  {:<12}  {}", rule.uuid, rule.rule_type, rule.name);
                    }
                }
            }
            RuleCommands::Show { uuid } => {
                let (_rule, xml) = read_rule(&cli.repo, &uuid)?;
                print!("{xml}");
            }
            RuleCommands::Validate => {
                let rules = validate_rule_repository(&cli.repo)?;
                println!("Validation passed: {} rule document(s).", rules.len());
            }
        },
    }

    Ok(())
}
