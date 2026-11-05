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

Git is the history engine. LawGit does not create a separate amendment-history file and does not commit automatically.

After editing the affected XML document files:

~~~sh
lawgit --repo legal-data validate
cd legal-data
git add laws/<UUID>.xml
git commit -m "道路交通法：改正内容を記述"
~~~

Use one commit for one amendment. When one amending act changes multiple legal documents, stage all affected XML files in the same commit. Record the amending act and the change summary in the commit message.

Use branches for proposed amendments and merge them when the proposal is adopted. Use Git tags when a named repository state must be retained.

## Current commands

| Command | Purpose |
|---|---|
| git law init (or lawgit init) | Initialize a Git-backed law repository and create laws/ |
| git law new (or lawgit new) | Create a new XML document with a generated UUID v4 |
| git law validate (or lawgit validate) | Validate XML syntax, required elements, UUID format, filenames, duplicate UUIDs, and body structure |
| git law list (or lawgit list) | List validated legal documents |
| git law show <UUID> (or lawgit show <UUID>) | Print a legal document's XML |

The --repo option selects the legal-data repository. It defaults to the current directory.

## Scope of this initial implementation

This version provides local repository initialization, document registration, XML validation, listing, and reading.

It does not yet import data from e-Gov, determine amendment text automatically, provide a graphical editor, or define a complete vocabulary for article/paragraph/item XML elements. The legal body is preserved as structured XML, while its detailed element vocabulary remains a separate specification task.
