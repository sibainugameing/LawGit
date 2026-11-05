# LawGit

LawGit is a Rust CLI for managing legal documents as structured XML files in a Git repository.

The data format and repository layout are defined by standard-jp.md and standard-en.md.

## Requirements

- Rust and Cargo
- Git

## Build

~~~sh
cargo build --release
~~~

The executables are created at target/release/lawgit and target/release/git-law. The latter enables Git's external subcommand syntax (`git law ...`). You can install both with:

~~~sh
cargo install --path .
~~~

## Git subcommand integration

LawGit supports Git's external subcommand mechanism. Install the package and ensure Cargo's bin directory is in your PATH:

~~~sh
cargo install --path .
# Usually ~/.cargo/bin should be in PATH
~~~

Then use LawGit as a Git subcommand:

~~~sh
git law init
git law new --name "道路交通法" --type "法律"
git law validate
git law list
git law show <UUID>
~~~

Git discovers the executable named `git-law` from PATH. LawGit does not modify or replace the Git executable.

The regular `lawgit` command remains available as well.

## Quick start

Create a separate repository for the legal documents:

~~~sh
mkdir legal-data
git law --repo legal-data init
~~~

Register a legal document. LawGit generates a UUID v4 and creates laws/<UUID>.xml:

~~~sh
git law --repo legal-data new \
  --name "道路交通法" \
  --type "法律" \
  --law-number "昭和三十五年法律第百五号" \
  --e-gov-id "335AC0000000105"
~~~

Edit the generated XML file to add the legal text. The body must use XML structural elements; it must not be stored as one plain-text value.

Validate and inspect the repository:

~~~sh
git law --repo legal-data validate
git law --repo legal-data list
git law --repo legal-data show <UUID>
~~~

## Git workflow

Git is the history engine. LawGit does not create a separate amendment-history file. Manual XML edits are committed manually; the `amend` command validates and commits an applied amendment automatically.

For manual edits, validate and commit the affected XML file yourself:

~~~sh
lawgit --repo legal-data validate
cd legal-data
git add laws/<UUID>.xml
git commit -m "道路交通法：改正内容を記述"
~~~

For amendments applied through LawGit, use `lawgit amend`. It validates the result and creates a commit automatically. The promulgation date of the amending law is used for both Git timestamps; its commencement date(s) are recorded as commit-message trailers.

### External application integration

An external editor can pass either a complete law XML document or a unified diff to LawGit. LawGit applies the change to the existing UUID file and commits only that file.

Replace the complete XML document:

~~~sh
lawgit --repo legal-data amend \
  --uuid <UUID> \
  --input amended-law.xml \
  --promulgated-date 2026-11-05 \
  --effective-date 2026-12-01 \
  --message "道路交通法の改正"
~~~

Read a complete XML document from standard input:

~~~sh
cat amended-law.xml | lawgit --repo legal-data amend \
  --uuid <UUID> --input - \
  --promulgated-date 2026-11-05 \
  --effective-date 2026-12-01 \
  --message "道路交通法の改正"
~~~

Apply a unified diff instead:

~~~sh
cat amendment.patch | lawgit --repo legal-data amend \
  --uuid <UUID> --patch - \
  --promulgated-date 2026-11-05 \
  --effective-date 2026-12-01 \
  --message "道路交通法の改正"
~~~

Supply `--effective-date` more than once when an amendment has multiple commencement dates. The date options accept ISO dates in `YYYY-MM-DD` format. The Git author and committer timestamps are set to 00:00 JST on the amendment's promulgation date.

This CLI interface is the integration boundary for external applications; an application can launch the process and communicate through files or standard input. No network API server is required.

Use one commit for one amendment. When one amending act changes multiple legal documents, stage all affected XML files in the same commit. Record the amending act and the change summary in the commit message.

Use branches for proposed amendments and merge them when the proposal is adopted. Use Git tags when a named repository state must be retained.

## User-authored rule documents

LawGit can manage user-authored rules and policy documents separately from legal documents. Rule documents are stored in `rules/<UUID>.xml`; they do not use the legal-document schema in `laws/`.

Create a rule interactively:

~~~sh
lawgit rule make
~~~

Enter the rule name and category, then enter the body. The body accepts Markdown text and is stored in a CDATA section, so ordinary Markdown characters can be edited directly in the XML file. Finish body input by entering a single dot (`.`) on its own line; enter two dots (`..`) to insert a literal dot line.

Manage existing rules:

~~~sh
lawgit rule list
lawgit rule show <UUID>
lawgit rule validate
~~~

The XML file path is shown after creation and in the list output. **Creating a rule automatically creates a Git commit** (message: `Create rule: <name>`). The commit includes only the new rule file, not unrelated staged changes.

Edit an existing rule with a text editor. Review and commit changes manually:

~~~sh
git diff -- rules/
git add rules/<UUID>.xml
git commit -m "Update rule: <rule name>"
~~~

Each rule receives an immutable UUID v4. Its name and category are metadata, not identifiers. Rule documents have their own XML root (`rule`) and validation; they are kept separate from legal documents.

The same commands are available through the Git subcommand:

~~~sh
git law rule make
git law rule list
git law rule show <UUID>
git law rule validate
~~~

## Current commands

| Command | Purpose |
|---|---|
| git law init (or lawgit init) | Initialize a Git-backed law repository and create laws/ |
| git law new (or lawgit new) | Create a new XML document with a generated UUID v4 |
| git law validate (or lawgit validate) | Validate XML syntax, required elements, UUID format, filenames, duplicate UUIDs, and body structure |
| git law list (or lawgit list) | List validated legal documents |
| git law show <UUID> (or lawgit show <UUID>) | Print a legal document's XML |
| git law amend (or lawgit amend) | Apply complete XML or a unified diff, validate, and commit the amendment |

The --repo option selects the legal-data repository. It defaults to the current directory.

## Scope of this initial implementation

This version provides local repository initialization, document registration, XML validation, listing, reading, and amendment application from complete XML or unified diff input. Amendment commits record the amending law's promulgation date in Git timestamps and commencement dates in commit-message trailers.

It does not yet import data from e-Gov, determine amendment text automatically, provide a graphical editor, or define a complete vocabulary for article/paragraph/item XML elements. The legal body is preserved as structured XML, while its detailed element vocabulary remains a separate specification task.
